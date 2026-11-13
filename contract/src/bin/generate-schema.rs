use std::{env, fs, path::Path};
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let write = env::args().any(|a| a == "--write");
    for kind in ["chart", "dashboard", "common"] {
        let text = serde_json::to_string_pretty(&facetra_contract::schema::schema(kind).unwrap())
            .unwrap()
            + "\n";
        let file = root.join(format!("Contracts/charting/v1/{kind}.schema.json"));
        if write {
            fs::write(&file, text).unwrap();
        } else {
            assert_eq!(
                fs::read_to_string(&file).unwrap(),
                text,
                "Schema drift: run Tools/check-contract.sh --write"
            );
        }
    }
    let file = root.join("Contracts/workspace/v1/metadata.schema.json");
    let text = serde_json::to_string_pretty(&facetra_contract::schema::workspace_metadata())
        .unwrap()
        + "\n";
    if write {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    } else {
        assert_eq!(
            fs::read_to_string(file).unwrap(),
            text,
            "Workspace schema drift: run Tools/check-contract.sh --write"
        );
    }
}
