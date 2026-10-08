use std::time::Duration;

use crate::ops::runtime::RuntimeCatalogOption;
use crate::runtime_status::models::{option, ModelCatalog, Query, Reason};
use crate::shell_path::LoginShellEnv;

pub(crate) fn query(executable: &str, env: &LoginShellEnv) -> Result<ModelCatalog, Reason> {
    let output = crate::runtime_status::models::run(Query {
        executable,
        args: &["models"],
        stdin: None,
        env,
        timeout: Duration::from_secs(15),
    })?;
    parse(&output)
}

fn parse(bytes: &[u8]) -> Result<ModelCatalog, Reason> {
    let output = std::str::from_utf8(bytes).map_err(|_| Reason::InvalidOutput)?;
    let mut lines = output.lines();
    lines
        .find(|line| line.trim() == "Available models")
        .ok_or(Reason::InvalidOutput)?;
    let mut models: Vec<RuntimeCatalogOption> = Vec::new();
    let mut default_model = None;
    for line in lines {
        let line = line.trim();
        if line.starts_with("Tip:") {
            break;
        }
        let Some((id, label)) = line.split_once(" - ") else {
            continue;
        };
        let id = id.trim();
        if id.is_empty()
            || id.starts_with('-')
            || id.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            continue;
        }
        let mut label = label.trim();
        if let Some((name, suffix)) = label.rsplit_once(" (") {
            if let Some(markers) = suffix.strip_suffix(')') {
                let markers: Vec<_> = markers.split(',').map(str::trim).collect();
                if markers
                    .iter()
                    .all(|marker| matches!(*marker, "current" | "default"))
                {
                    if markers.contains(&"default") {
                        default_model = Some(id.to_owned());
                    }
                    label = name.trim();
                }
            }
        }
        if label.is_empty() || models.iter().any(|model| model.value == id) {
            continue;
        }
        let mut model = option(id.to_owned(), label.to_owned(), None);
        model.supported_efforts = Some(Vec::new());
        models.push(model);
    }
    if models.is_empty() {
        return Err(Reason::EmptyCatalog);
    }
    Ok(ModelCatalog {
        models,
        default_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_rows_without_a_fixed_model_list() {
        let output = "Available models\n\nauto - Auto (current, default)\ncomposer-2.5 - Composer 2.5\ngrok-4.7-high-fast - Grok 4.7 High Fast\nfuture-model[context=1m,effort=high] - Future Model (Thinking)\ncomposer-2.5 - Duplicate\n\nTip: use --model <id> to switch.\nexample - Not a model\n";
        let catalog = parse(output.as_bytes()).unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [
                "auto",
                "composer-2.5",
                "grok-4.7-high-fast",
                "future-model[context=1m,effort=high]"
            ]
        );
        assert_eq!(catalog.default_model.as_deref(), Some("auto"));
        assert_eq!(catalog.models[0].label, "Auto");
        assert_eq!(catalog.models[3].label, "Future Model (Thinking)");
        assert!(catalog
            .models
            .iter()
            .all(|model| model.supported_efforts.as_ref().is_some_and(Vec::is_empty)));
        let updated =
            parse(b"Available models\nnew-account-model - New Model (current)\n").unwrap();
        assert_eq!(updated.models[0].value, "new-account-model");
        assert_eq!(updated.models[0].label, "New Model");
        assert!(updated.default_model.is_none());
    }

    #[test]
    fn rejects_failed_empty_or_invalid_output() {
        assert_eq!(
            parse(b"Authentication required"),
            Err(Reason::InvalidOutput)
        );
        assert_eq!(
            parse(b"Available models\n\nTip: use --model <id>\n"),
            Err(Reason::EmptyCatalog)
        );
        assert_eq!(
            parse(b"Available models\n--flag - Bad\nwhite space - Bad\n"),
            Err(Reason::EmptyCatalog)
        );
        assert_eq!(parse(&[0xff]), Err(Reason::InvalidOutput));
    }
}
