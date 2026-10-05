use super::*;
pub fn option(value: &str, label: &str, description: &str) -> RuntimeCatalogOption {
    RuntimeCatalogOption {
        value: value.into(),
        label: label.into(),
        description: Some(description.into()),
        supported_efforts: None,
    }
}

pub fn plain_option(value: &str, label: &str) -> RuntimeCatalogOption {
    RuntimeCatalogOption {
        value: value.into(),
        label: label.into(),
        description: None,
        supported_efforts: None,
    }
}

pub fn default_model_option() -> RuntimeCatalogOption {
    option("", "default", "Use the agent's own default model.")
}

pub fn default_effort() -> RuntimeCatalogOption {
    option(
        "",
        "default",
        "Use the agent's own default effort; no flag passed.",
    )
}

pub fn common_efforts() -> Vec<RuntimeCatalogOption> {
    vec![
        default_effort(),
        option("low", "Low", "Fast responses with lighter reasoning."),
        option("medium", "Medium", "Balances speed and reasoning depth."),
        option(
            "high",
            "High",
            "Greater reasoning depth for complex problems.",
        ),
        option(
            "xhigh",
            "Extra high",
            "Extra reasoning depth for complex problems.",
        ),
    ]
}
