use serde_json::Value;

pub(crate) fn assert_golden(name: &str, value: Value) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/expectations")
        .join(format!("catalog-{name}.json"));
    let actual = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
    if std::env::var("RUNNER_UPDATE_CATALOG_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(
        actual,
        std::fs::read_to_string(path).unwrap(),
        "app catalog {name} changed"
    );
}

thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Vec<Value>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn capture(run: impl FnOnce()) -> Vec<Value> {
    CAPTURE.with_borrow_mut(|capture| *capture = Some(Vec::new()));
    run();
    CAPTURE.with_borrow_mut(|capture| capture.take().unwrap())
}

pub(crate) fn record(value: Value) {
    CAPTURE.with_borrow_mut(|capture| {
        if let Some(capture) = capture {
            capture.push(value);
        }
    });
}
