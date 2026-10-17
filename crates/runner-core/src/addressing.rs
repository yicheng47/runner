use crate::model::Recipients;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub recipients: Recipients,
    pub text: String,
}

pub fn resolve(text: &str, explicit: &[String], roster: &[String]) -> crate::Result<Resolved> {
    let targets: Vec<String> = if explicit.is_empty() {
        text.split_whitespace()
            .take_while(|token| token.starts_with('@'))
            .map(|token| token[1..].to_owned())
            .collect()
    } else {
        explicit.to_vec()
    };
    for target in &targets {
        if target == "human" || !roster.contains(target) {
            return Err(crate::Error::msg(format!(
                "@{target} is not a message recipient; valid handles: {}",
                roster
                    .iter()
                    .map(|handle| format!("@{handle}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(Resolved {
        recipients: Recipients::new(targets),
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_only_leading_mentions_and_explicit_recipients_win() {
        let roster = vec!["coder".into(), "qa".into(), "reviewer".into()];
        for (text, explicit, expected) in [
            ("hello @qa", vec![], vec![]),
            ("@coder hello", vec![], vec!["coder"]),
            (" @coder @qa hello", vec![], vec!["coder", "qa"]),
            ("@human hello", vec!["qa"], vec!["qa"]),
            (
                "@missing hello",
                vec!["qa", "qa", "reviewer"],
                vec!["qa", "reviewer"],
            ),
        ] {
            let explicit: Vec<String> = explicit.into_iter().map(str::to_owned).collect();
            let result = resolve(text, &explicit, &roster).unwrap();
            assert_eq!(result.recipients.as_slice(), expected);
            assert_eq!(result.text, text);
        }
        for (text, explicit) in [
            ("hello", vec!["human".into()]),
            ("@human hello", vec![]),
            ("@missing hello", vec![]),
        ] {
            let error = resolve(text, &explicit, &roster).unwrap_err().to_string();
            for handle in &roster {
                assert!(error.contains(handle));
            }
        }
    }
}
