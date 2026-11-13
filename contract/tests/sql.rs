use facetra_contract::sql::*;
use serde_json::Value;
use std::collections::BTreeMap;
#[test]
fn scanner_preserves_literals_comments_and_binds_every_occurrence() {
    let params = BTreeMap::from([("x".into(), Value::from("'; DROP TABLE input; --"))]);
    let query = "/* a /* nested */ b */ SELECT '$x;?', \"$x\", $x, $x -- $missing\n";
    let compiled = compile(query, &params).unwrap();
    assert_eq!(
        compiled.sql,
        "/* a /* nested */ b */ SELECT '$x;?', \"$x\", $1, $2 -- $missing\n"
    );
    assert_eq!(compiled.values.len(), 2);
    for bad in [
        "SELECT 1; SELECT 2",
        "DELETE FROM x",
        "SELECT ?",
        "SELECT $$x$$",
        "SELECT $1",
        "SELECT 'x",
        "/* unclosed",
    ] {
        assert!(compile(bad, &params).is_err(), "{bad}");
    }
}
