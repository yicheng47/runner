//! pi adapter: `pi --offline --list-models` prints an aligned text table.

use std::time::Duration;

use super::{option, ModelCatalog, Query, Reason};
use crate::ops::runtime::RuntimeCatalogOption;
use crate::shell_path::LoginShellEnv;

const TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn query(executable: &str, env: &LoginShellEnv) -> Result<ModelCatalog, Reason> {
    let output = super::run(Query {
        executable,
        args: &["--offline", "--list-models"],
        stdin: None,
        env,
        timeout: TIMEOUT,
    })?;
    parse(&output)
}

fn parse(bytes: &[u8]) -> Result<ModelCatalog, Reason> {
    let Ok(output) = std::str::from_utf8(bytes) else {
        return Err(Reason::InvalidOutput);
    };
    let mut lines = output.lines();
    let Some(header) = lines.find(|line| {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        columns.first() == Some(&"provider") && columns.get(1) == Some(&"model")
    }) else {
        return Err(Reason::InvalidOutput);
    };
    if header.split_whitespace().count() < 2 {
        return Err(Reason::InvalidOutput);
    }

    let mut models: Vec<RuntimeCatalogOption> = Vec::new();
    for line in lines {
        let mut columns = line.split_whitespace();
        let (Some(provider), Some(model)) = (columns.next(), columns.next()) else {
            continue;
        };
        let value = format!("{provider}/{model}");
        if models.iter().any(|known| known.value == value) {
            continue;
        }
        models.push(option(value.clone(), value, None));
    }
    if models.is_empty() {
        return Err(Reason::EmptyCatalog);
    }
    Ok(ModelCatalog {
        models,
        default_model: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str =
        "provider      model                         context  max-out  thinking  images\n\
anthropic     claude-fable-5                1M       128K     yes       yes   \n\
deepseek      deepseek-v4-pro               1M       384K     yes       no    \n\
openai-codex  gpt-5.5                       272K     128K     yes       yes   \n";

    #[test]
    fn parses_provider_model_columns_from_recorded_table() {
        let catalog = parse(TABLE.as_bytes()).unwrap();
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.value.as_str())
                .collect::<Vec<_>>(),
            [
                "anthropic/claude-fable-5",
                "deepseek/deepseek-v4-pro",
                "openai-codex/gpt-5.5",
            ]
        );
        assert!(catalog
            .models
            .iter()
            .all(|model| model.label == model.value));
    }

    #[test]
    fn rejects_garbage_and_empty_tables() {
        assert_eq!(parse(b"not a table"), Err(Reason::InvalidOutput));
        assert_eq!(
            parse(b"provider model context max-out thinking images\n"),
            Err(Reason::EmptyCatalog)
        );
    }
}
