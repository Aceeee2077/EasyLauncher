use easylauncher_lib::storage;
#[test]
fn public_storage_api_rejects_traversal_and_reserved_names() {
    let root = std::path::Path::new("managed-servers");
    for name in [
        "../outside",
        "..\\outside",
        "NUL",
        "CON",
        "world.",
        "file:stream",
    ] {
        assert!(storage::child(root, name).is_err(), "accepted {name}");
    }
    let id = uuid::Uuid::new_v4().to_string();
    assert!(storage::managed(root, &id).is_ok());
}
