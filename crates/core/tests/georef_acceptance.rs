//! Synthetic location -> native save/reopen -> IFC output, checked against independent PROJ numbers.
//! Regenerate/check the oracle with `python ci/accept_georef.py CLI OUTPUT` (PROJ 9.5.1).

#[cfg(test)]
mod tests {
    use bimcraft_core::engine::Session;
    use bimcraft_core::step::{self, File, V};
    use serde_json::{Value, json};

    fn number(value: &V) -> f64 {
        match value {
            V::Real(value) => *value,
            V::Int(value) => *value as f64,
            value => panic!("expected number, got {value:?}"),
        }
    }

    fn only<'a>(file: &'a File, name: &str) -> &'a step::Entity {
        let entities: Vec<_> = file.entities.values().filter(|entity| entity.name == name).collect();
        assert_eq!(entities.len(), 1, "one {name}");
        entities[0]
    }

    fn close(actual: f64, expected: f64, tolerance: f64, label: &str) {
        assert!((actual - expected).abs() <= tolerance, "{label}: {actual} != {expected} (tolerance {tolerance})");
    }

    #[test]
    fn saved_sites_export_independent_proj_positions_scale_and_true_north_in_both_hemispheres() {
        let cases: Vec<Value> = serde_json::from_str(include_str!("../../../ci/georef_cases.json")).unwrap();
        for case in cases {
            let directory = tempfile::tempdir().unwrap();
            let native = directory.path().join("site.bimcraft");
            let ifc = directory.path().join("site.ifc");
            let location = &case["location"];
            let label = case["name"].as_str().unwrap();
            let oracle = |key: &str| case[key].as_f64().unwrap();
            let mut session = Session::new();
            session.execute("wall", &json!({"points": [[0, 0], [10000, 0]], "height": 3000, "type": "Interior - 100mm"})).unwrap();
            session.execute("geolocate", location).unwrap();
            session.execute("save", &json!({"path": native})).unwrap();

            let mut reopened = Session::new();
            let opened = reopened.execute("open", &json!({"path": native})).unwrap();
            assert!(opened["left_out"].as_array().is_none_or(Vec::is_empty), "{label}: {opened}");
            assert_eq!(reopened.model.location, session.model.location, "saved location");
            let before = reopened.execute("geolocate", &json!({})).unwrap();
            assert!(reopened.execute("geolocate", &json!({"crs": 4326})).unwrap_err().contains("projected"));
            assert_eq!(reopened.execute("geolocate", &json!({})).unwrap(), before, "invalid CRS must preserve the saved location");
            reopened.execute("save", &json!({"path": ifc})).unwrap();
            let file = step::parse(&std::fs::read(&ifc).unwrap()).unwrap();
            let site = only(&file, "IFCSITE");
            for (index, key) in [(9, "latitude"), (10, "longitude")] {
                let V::List(parts) = &site.args[index] else { panic!("compound angle") };
                let degrees: f64 = parts.iter().zip([1.0, 60.0, 3600.0, 3.6e9]).map(|(part, unit)| number(part) / unit).sum();
                close(degrees, location[key].as_f64().unwrap(), 1e-9, label);
            }
            close(number(&site.args[11]), 12500.0, 1e-6, label);
            let map = only(&file, "IFCMAPCONVERSION");
            close(number(&map.args[2]), oracle("easting"), 1e-3, label);
            close(number(&map.args[3]), oracle("northing"), 1e-3, label);
            close(number(&map.args[4]), 12.5, 1e-9, label);
            close(number(&map.args[7]), oracle("scale") / 1000.0, 1e-10, label);
            let angle = location["true_north"].as_f64().unwrap().to_radians();
            let rotation = oracle("convergence").to_radians() - angle;
            close(number(&map.args[5]), rotation.cos(), 2e-8, label);
            close(number(&map.args[6]), rotation.sin(), 2e-8, label);
            let context = only(&file, "IFCGEOMETRICREPRESENTATIONCONTEXT");
            let V::Ref(north) = context.args[5] else { panic!("TrueNorth direction") };
            let V::List(ratios) = &file.entities[&north].args[0] else { panic!("direction ratios") };
            close(number(&ratios[0]), -angle.sin(), 1e-9, label);
            close(number(&ratios[1]), angle.cos(), 1e-9, label);
            // A true-north unit ray transformed to the grid must include convergence exactly once.
            let (x, y) = (number(&ratios[0]), number(&ratios[1]));
            let (c, s) = (number(&map.args[5]), number(&map.args[6]));
            close(x * c - y * s, -oracle("convergence").to_radians().sin(), 2e-8, label);
            close(x * s + y * c, oracle("convergence").to_radians().cos(), 2e-8, label);

            let mut imported = Session::new();
            imported.execute("open", &json!({"path": ifc})).unwrap();
            let actual = imported.execute("geolocate", &json!({})).unwrap();
            for key in ["latitude", "longitude", "true_north", "elevation"] {
                close(actual[key].as_f64().unwrap(), location[key].as_f64().unwrap(), 1e-6, label);
            }
        }
    }
}
