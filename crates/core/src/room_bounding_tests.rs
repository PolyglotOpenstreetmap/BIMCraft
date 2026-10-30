//! A wall's Room Bounding, as Revit's: off, rooms run through the wall; floors made from walls are the same either way.

use serde_json::{Value, json};

use crate::engine::Session;
use crate::model::Model;

fn run(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c} {p}: {e}"))
}

/// A 10 m x 6 m box of 300 mm walls on their centre lines, split in two by a wall at x = 5000, with a room in each half.
/// Returns the dividing wall's id.
fn open_plan(s: &mut Session) -> u64 {
    run(s, "wall", json!({"points": [[0, 0], [10000, 0], [10000, 6000], [0, 6000]], "closed": true, "type": "Exterior - 300mm"}));
    let divider = run(s, "wall", json!({"points": [[5000, 0], [5000, 6000]], "type": "Exterior - 300mm"}));
    run(s, "room", json!({"point": [2500, 3000], "name": "Living"}));
    run(s, "room", json!({"point": [7500, 3000], "name": "Dining"}));
    divider["ids"][0].as_u64().unwrap_or_default()
}

/// Every room's area, smallest first.
fn areas(s: &mut Session) -> Vec<f64> {
    let rooms = run(s, "elements", json!({"category": "room"}));
    let mut list: Vec<f64> = rooms["elements"].as_array().unwrap().iter().filter_map(|e| e["area_m2"].as_f64()).collect();
    list.sort_by(f64::total_cmp);
    list
}

fn near(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{got:?} isn't {want:?}");
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.006), "{got:?} isn't {want:?}");
}

#[test]
fn a_wall_that_does_not_bound_rooms_lets_them_run_through() {
    let mut s = Session::new();
    let divider = open_plan(&mut s);
    near(&areas(&mut s), &[30.0, 30.0]);
    run(&mut s, "modify", json!({"id": divider, "room_bounding": false}));
    near(&areas(&mut s), &[60.0, 60.0]);
    // On again, they split again; and each step is one undo.
    run(&mut s, "modify", json!({"id": divider, "room_bounding": true}));
    near(&areas(&mut s), &[30.0, 30.0]);
    run(&mut s, "undo", json!({}));
    near(&areas(&mut s), &[60.0, 60.0]);
    run(&mut s, "redo", json!({}));
    near(&areas(&mut s), &[30.0, 30.0]);
}

#[test]
fn it_holds_at_the_finish_faces_too_and_a_room_may_lie_in_the_wall() {
    let mut s = Session::new();
    let divider = open_plan(&mut s);
    run(&mut s, "room_area", json!({"at": "finish"}));
    // Between the finish faces: (5000 - 300) x (6000 - 300).
    near(&areas(&mut s), &[26.79, 26.79]);
    // A point inside a bounding wall's body is refused as a room; once the wall stops bounding it is only more floor.
    assert!(s.execute("room", &json!({"point": [5000, 3000]})).is_err());
    run(&mut s, "modify", json!({"id": divider, "room_bounding": false}));
    near(&areas(&mut s), &[55.29, 55.29]);
    run(&mut s, "room", json!({"point": [5000, 3000], "name": "In the wall"}));
}

#[test]
fn floors_made_from_walls_do_not_care() {
    let floor = |off: bool| {
        let mut s = Session::new();
        let divider = open_plan(&mut s);
        if off {
            run(&mut s, "modify", json!({"id": divider, "room_bounding": false}));
        }
        run(&mut s, "floor", json!({}))
    };
    assert_eq!(floor(true), floor(false));
}

#[test]
fn a_separation_line_still_splits_where_a_wall_no_longer_does() {
    let mut s = Session::new();
    let divider = open_plan(&mut s);
    run(&mut s, "modify", json!({"id": divider, "room_bounding": false}));
    run(&mut s, "room_separator", json!({"points": [[5000, 0], [5000, 6000]]}));
    near(&areas(&mut s), &[30.0, 30.0]);
}

#[test]
fn the_flag_is_saved_and_a_file_without_it_reads_as_bounding() {
    let mut s = Session::new();
    let divider = open_plan(&mut s);
    // A project that never turned it off writes no such field.
    let file = serde_json::to_string(&*s.model).unwrap();
    assert!(!file.contains("room_bounding"), "{file}");
    run(&mut s, "modify", json!({"id": divider, "room_bounding": false}));
    let path = std::env::temp_dir().join(format!("bimcraft-test-{}-room-bounding.bimcraft", std::process::id()));
    run(&mut s, "save", json!({"path": path}));
    let mut t = Session::new();
    run(&mut t, "open", json!({"path": path}));
    near(&areas(&mut t), &[60.0, 60.0]);
    let _ = std::fs::remove_file(path);
    // An older file has no field, and nothing is left out of it.
    let mut file = serde_json::to_value(&*s.model).unwrap();
    let text = file.to_string();
    assert!(text.contains("room_bounding"));
    file = serde_json::from_str(&text.replace("\"room_bounding\":false,", "").replace(",\"room_bounding\":false", "")).unwrap();
    let (m, left_out) = Model::read(&file.to_string()).unwrap();
    assert_eq!(left_out.len(), 0);
    let mut u = Session::new();
    *u.model_mut() = m;
    near(&areas(&mut u), &[30.0, 30.0]);
}

#[test]
fn a_wall_can_be_made_not_bounding_and_only_true_or_false_is_taken() {
    let mut s = Session::new();
    run(&mut s, "wall", json!({"points": [[0, 0], [10000, 0], [10000, 6000], [0, 6000]], "closed": true, "type": "Exterior - 300mm"}));
    run(&mut s, "wall", json!({"points": [[5000, 0], [5000, 6000]], "type": "Exterior - 300mm", "room_bounding": false}));
    run(&mut s, "room", json!({"point": [2500, 3000]}));
    near(&areas(&mut s), &[60.0]);
    let divider = run(&mut s, "elements", json!({"category": "wall"}))["elements"]
        .as_array()
        .and_then(|w| w.iter().find(|e| e["room_bounding"] == json!(false)).and_then(|e| e["id"].as_u64()))
        .unwrap_or_default();
    assert_ne!(divider, 0, "the element's JSON shows the flag when it is off");
    for bad in [json!("no"), json!(0), json!(null), json!([true])] {
        assert!(s.execute("wall", &json!({"points": [[0, 0], [1000, 0]], "room_bounding": bad})).is_err(), "{bad}");
    }
    assert!(s.execute("modify", &json!({"id": divider, "room_bounding": "no"})).is_err());
    near(&areas(&mut s), &[60.0]);
}
