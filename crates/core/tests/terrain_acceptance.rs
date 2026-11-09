//! Synthetic acceptance of the terrain commands used by CLI and MCP clients.
#[cfg(test)]
mod tests {
    use bimcraft_core::engine::Session;
    use bimcraft_core::model::FORMAT;
    use serde_json::{Value, json};

    fn run(s: &mut Session, command: &str, params: Value) -> Value {
        s.execute(command, &params).unwrap_or_else(|error| panic!("{command} {params}: {error}"))
    }

    fn terrain(s: &mut Session) -> Value {
        run(
            s,
            "toposurface",
            json!({
                "name": "Synthetic slope", "interval": 250,
                "points": [[0,0,0], [10000,0,1000], [10000,10000,1000], [0,10000,0]],
                "breaklines": [[[0,5000,0],[10000,5000,1000]]],
                "boundary": [[0,0],[10000,0],[10000,10000],[0,10000]]
            }),
        )["id"]
            .clone()
    }

    fn verify_pad(s: &mut Session, ground: Value, pad: Value) {
        let result = run(s, "cut_fill", json!({"terrain": ground, "pads": [pad]}));
        assert_eq!(result["totals"], json!({"cut_m3": 6.4, "fill_m3": 6.4, "net_m3": 0.0}));
        assert_eq!(result["pads"][0]["covered_m2"], 64.0);
        assert_eq!(result["pads"][0]["area_m2"], 64.0);
        let heights = run(s, "height_at", json!({"id": ground, "points": [[1000,5000],[9000,5000],[-1,0]]}));
        assert_eq!(heights["heights"].as_array().unwrap().len(), 3);
        for (height, expected) in heights["heights"].as_array().unwrap().iter().take(2).zip([100.0, 900.0]) {
            assert!((height.as_f64().unwrap() - expected).abs() < 1e-6, "{heights}");
        }
        assert!(heights["heights"][2].is_null());
    }

    #[test]
    fn survey_pad_cut_fill_save_and_reopen_preserve_quantities_and_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        let ground = terrain(&mut s);
        let pad = run(&mut s, "pad", json!({"name": "Synthetic pad", "boundary": [[1000,1000],[9000,1000],[9000,9000],[1000,9000]], "offset": 500, "thickness": 200}))["id"].clone();
        verify_pad(&mut s, ground.clone(), pad.clone());
        let before = run(&mut s, "elements", json!({}));
        let bad = s.execute("pad", &json!({"boundary": [[0,0],[1000,0],[1000,1000]], "thickness": -1})).unwrap_err();
        assert!(bad.contains("thickness"), "{bad}");
        assert_eq!(run(&mut s, "elements", json!({})), before, "failed creation leaves the project intact");
        let path = dir.path().join("survey.bimcraft");
        run(&mut s, "save", json!({"path": path}));
        let mut reopened = Session::new();
        let opened = run(&mut reopened, "open", json!({"path": path}));
        assert!(opened["left_out"].as_array().is_none_or(Vec::is_empty), "{opened}");
        assert_eq!(run(&mut reopened, "elements", json!({})), before);
        verify_pad(&mut reopened, ground, pad);
    }

    #[test]
    fn a_constrained_valley_survives_native_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        run(
            &mut s,
            "toposurface",
            json!({
                "points": [[0,0,0],[12000,0,2000],[10000,10000,0],[0,8000,2000]],
                "breaklines": [[[0,0,0],[10000,10000,0]]]
            }),
        );
        let query = json!({"points": [[2500,2500],[5000,5000],[7500,7500]]});
        assert_eq!(run(&mut s, "height_at", query.clone())["heights"], json!([0.0, 0.0, 0.0]));
        let path = dir.path().join("valley.bimcraft");
        run(&mut s, "save", json!({"path": path}));
        run(&mut s, "open", json!({"path": path}));
        assert_eq!(run(&mut s, "height_at", query)["heights"], json!([0.0, 0.0, 0.0]));
    }

    #[test]
    fn pre_site_schema_opens_then_accepts_terrain_without_losing_unknown_field_report() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        let path = dir.path().join("legacy.bimcraft");
        run(&mut s, "save", json!({"path": path}));
        let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        file["format"] = json!(17);
        file["future_survey_annotation"] = json!("synthetic");
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let opened = run(&mut s, "open", json!({"path": path}));
        assert!(opened["left_out"].to_string().contains("future_survey_annotation"), "{opened}");
        assert_eq!(s.model.format, FORMAT);
        terrain(&mut s);
        run(&mut s, "save", json!({"path": path}));
        run(&mut s, "open", json!({"path": path}));
        assert_eq!(run(&mut s, "elements", json!({"category": "toposurface"}))["count"], 1);
    }

    #[test]
    fn planar_survey_and_pad_export_and_reopen_as_terrain_and_pad() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new();
        terrain(&mut s);
        run(
            &mut s,
            "pad",
            json!({"name": "Synthetic pad", "boundary": [[1000,1000],[9000,1000],[9000,9000],[1000,9000]], "offset": 500, "thickness": 200}),
        );
        let original_terrain = run(&mut s, "elements", json!({"category": "toposurface"}))["elements"][0].clone();
        let original_pad = run(&mut s, "elements", json!({"category": "pad"}))["elements"][0].clone();
        assert!(original_terrain["guid"].as_str().is_some_and(|guid| !guid.is_empty()));
        assert!(original_pad["guid"].as_str().is_some_and(|guid| !guid.is_empty()));
        let path = dir.path().join("survey.ifc");
        run(&mut s, "save", json!({"path": path}));
        let mut reopened = Session::new();
        let opened = run(&mut reopened, "open", json!({"path": path}));
        assert_eq!(opened["skipped"], json!([]));
        let ground = run(&mut reopened, "elements", json!({"category": "toposurface"}));
        let pads = run(&mut reopened, "elements", json!({"category": "pad"}));
        assert_eq!(ground["count"], 1);
        assert_eq!(pads["count"], 1);
        assert_eq!(ground["elements"][0]["guid"], original_terrain["guid"]);
        assert_eq!(pads["elements"][0]["guid"], original_pad["guid"]);
        verify_pad(&mut reopened, ground["elements"][0]["id"].clone(), pads["elements"][0]["id"].clone());
    }
}
