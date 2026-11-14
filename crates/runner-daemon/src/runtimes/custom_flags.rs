use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use runner_core::protocol::runtime::{CustomFlagsValidationError, RuntimeLaunchOptions};
use runner_core::protocol::RuntimeCatalog;

use crate::model::{CodexSpeed, Runtime};
use crate::router::runtime::{ForkPlan, PermissionMode};

pub(crate) fn parse_custom_flags(raw: &str) -> Result<Vec<String>, CustomFlagsValidationError> {
    let normalized = normalize_newlines(raw);
    let mut lex = shlex::Shlex::new(&normalized);
    let words: Vec<String> = lex.by_ref().collect();
    if lex.had_error {
        Err(CustomFlagsValidationError {
            code: "unclosed_quote".into(),
            message: "A quote isn't closed.".into(),
        })
    } else {
        Ok(words)
    }
}

pub(crate) fn parse_lossy(raw: &str) -> Vec<String> {
    parse_custom_flags(raw).unwrap_or_else(|_| {
        let normalized = normalize_newlines(raw);
        shlex::Shlex::new(&normalized).collect()
    })
}

pub(crate) fn format_preview_args(args: &[String]) -> String {
    shlex::try_join(args.iter().map(String::as_str)).unwrap_or_else(|_| args.join(" "))
}

fn normalize_newlines(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn command_name(runtime: Runtime) -> &'static str {
    crate::runtimes::adapter(runtime)
        .catalog()
        .map_or_else(|| runtime.key(), |catalog| catalog.command)
}

#[derive(Debug, Default)]
pub(crate) struct ReservedFlags {
    pub(crate) flags: HashSet<String>,
    pub(crate) config_keys: HashSet<String>,
}

impl ReservedFlags {
    fn collect_args(&mut self, args: &[String]) {
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if matches!(arg.as_str(), "-c" | "--config") {
                if let Some(spec) = args.get(i + 1) {
                    self.config_keys.extend(extract_toml_key_paths(spec));
                    i += 2;
                    continue;
                }
            } else if let Some(spec) = arg
                .strip_prefix("--config=")
                .or_else(|| arg.strip_prefix("-c="))
            {
                self.config_keys.extend(extract_toml_key_paths(spec));
                i += 1;
                continue;
            } else if arg.starts_with('-') {
                let name = flag_name(arg);
                if name != "--add-dir" {
                    self.flags.insert(name.to_owned());
                }
            }
            i += 1;
        }
    }

    pub(crate) fn supports_config_keys(&self) -> bool {
        !self.config_keys.is_empty()
    }
}

fn flag_name(token: &str) -> &str {
    token.split_once('=').map_or(token, |(name, _)| name)
}

pub(crate) fn sample_prior_key(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::OpenCode => "ses_00000000000000000000000000",
        _ => "00000000-0000-0000-0000-000000000000",
    }
}

struct SampleLaunchContext {
    model: Option<String>,
    effort: Option<String>,
    env: BTreeMap<String, String>,
}

fn catalog_launch_samples(catalog: &RuntimeCatalog) -> Vec<SampleLaunchContext> {
    let catalog_efforts: Vec<&str> = catalog
        .efforts
        .iter()
        .map(|option| option.value.as_str())
        .filter(|value| !value.is_empty())
        .collect();
    let mut seen_efforts = HashSet::new();
    let mut samples = Vec::new();
    let mut fallback_model = None;

    for model in &catalog.models {
        if model.value.is_empty() {
            continue;
        }
        if let Some(supported) = &model.supported_efforts {
            for effort in supported {
                if !effort.is_empty() && seen_efforts.insert(effort.clone()) {
                    samples.push(SampleLaunchContext {
                        model: Some(model.value.clone()),
                        effort: Some(effort.clone()),
                        env: BTreeMap::new(),
                    });
                }
            }
        } else if fallback_model.is_none() {
            fallback_model = Some(model.value.clone());
        }
    }

    let any_model = fallback_model.or_else(|| {
        catalog
            .models
            .iter()
            .find(|model| !model.value.is_empty())
            .map(|model| model.value.clone())
    });
    if let Some(model) = any_model {
        for effort in catalog_efforts {
            if seen_efforts.insert(effort.to_owned()) {
                samples.push(SampleLaunchContext {
                    model: Some(model.clone()),
                    effort: Some(effort.to_owned()),
                    env: BTreeMap::new(),
                });
            }
        }
    }
    samples
}

fn sample_launch_contexts(runtime: Runtime, scratch_dir: &Path) -> Vec<SampleLaunchContext> {
    let mut contexts = Vec::new();
    if let Some(catalog) = crate::runtimes::adapter(runtime).catalog() {
        contexts.extend(catalog_launch_samples(&catalog));
    }
    if runtime == Runtime::Grok {
        let grok_home = scratch_dir.join("grok-reserved-sample");
        if std::fs::create_dir_all(&grok_home).is_ok()
            && std::fs::write(
                grok_home.join("models_cache.json"),
                r#"{"models":{"grok-sample":{"info":{"reasoning_efforts":[{"id":"high"}]}}}}"#,
            )
            .is_ok()
        {
            let mut env = BTreeMap::new();
            env.insert("GROK_HOME".into(), grok_home.to_string_lossy().into_owned());
            contexts.push(SampleLaunchContext {
                model: Some("grok-sample".into()),
                effort: Some("high".into()),
                env,
            });
        }
    }
    contexts
}

pub(crate) fn reserved_flags(runtime: Runtime) -> ReservedFlags {
    let mut reserved = ReservedFlags::default();
    if runtime == Runtime::Shell {
        return reserved;
    }

    let adapter = crate::runtimes::adapter(runtime);
    let scratch = tempfile::tempdir().ok();
    let app_data_dir = scratch
        .as_ref()
        .map_or_else(|| Path::new("/tmp"), |dir| dir.path());
    if let Some(hooks) = adapter.status_hooks() {
        hooks.install(app_data_dir);
    }

    let prior = sample_prior_key(runtime);
    for mission in [false, true] {
        for bypass in [false, true] {
            for status_hooks in [false, true] {
                for remote_plugin_catalog in [false, true] {
                    let options = RuntimeLaunchOptions {
                        bypass,
                        status_hooks,
                        remote_plugin_catalog,
                        custom_flags: String::new(),
                    };
                    for prior_key in [None, Some(prior)] {
                        let (args, _) = crate::session::manager::spawn_spec::compose_preview_args(
                            runtime,
                            mission,
                            &options,
                            app_data_dir,
                            Default::default(),
                            crate::session::manager::spawn_spec::PreviewArgsOverrides {
                                prior_key,
                                codex_speed: Some(CodexSpeed::Fast),
                                system_prompt: Some("<system prompt>"),
                                ..Default::default()
                            },
                        );
                        reserved.collect_args(&args);
                    }
                }
            }
        }
    }

    // Collect effort/config flags that only trigger for supported model+effort contexts
    // (for example Antigravity's ANTIGRAVITY_EFFORTS filter or Grok's cached model efforts).
    for sample in sample_launch_contexts(runtime, app_data_dir) {
        let options = RuntimeLaunchOptions {
            bypass: true,
            status_hooks: true,
            remote_plugin_catalog: true,
            custom_flags: String::new(),
        };
        for mission in [false, true] {
            for prior_key in [None, Some(prior)] {
                let (args, _) = crate::session::manager::spawn_spec::compose_preview_args(
                    runtime,
                    mission,
                    &options,
                    app_data_dir,
                    sample.env.clone(),
                    crate::session::manager::spawn_spec::PreviewArgsOverrides {
                        model: sample.model.as_deref(),
                        effort: sample.effort.as_deref(),
                        prior_key,
                        codex_speed: Some(CodexSpeed::Fast),
                        system_prompt: Some("<system prompt>"),
                    },
                );
                reserved.collect_args(&args);
            }
        }
    }

    let perms = adapter.permissions();
    for mode in perms
        .offered
        .iter()
        .copied()
        .chain([PermissionMode::Bypass])
    {
        reserved.collect_args(&perms.mode_args(mode));
    }
    reserved.collect_args(&perms.mission_args());
    for &(flag, _) in perms.strip_flags {
        if flag != "--add-dir" {
            reserved.flags.insert(flag.to_owned());
        }
    }
    if let Some(variadic) = perms.variadic_flag {
        if variadic != "--add-dir" {
            reserved.flags.insert(variadic.to_owned());
        }
    }

    reserved.collect_args(&adapter.resume_plan(Some(prior)).args);
    if let Some(ForkPlan::Direct(plan)) = adapter.fork_plan(prior, "fork") {
        reserved.collect_args(&plan.args);
    }

    reserved
}

pub(crate) fn check_reserved_flags(
    runtime: Runtime,
    words: &[String],
) -> Result<(), CustomFlagsValidationError> {
    let reserved = reserved_flags(runtime);
    if let Some(conflict) = first_reserved_conflict(words, &reserved) {
        return Err(CustomFlagsValidationError {
            code: "reserved_flag".into(),
            message: format!("Runner already sets {conflict}."),
        });
    }
    Ok(())
}

pub(crate) fn validate_save(
    runtime: Runtime,
    raw: &str,
    fetch_help: impl FnOnce() -> Option<HashSet<String>>,
) -> Result<(Vec<String>, Option<String>), CustomFlagsValidationError> {
    let words = parse_custom_flags(raw)?;
    if words.is_empty() {
        return Ok((words, None));
    }
    check_reserved_flags(runtime, &words)?;
    let listed = fetch_help();
    let warning = help_flag_warning(runtime, &words, listed.as_ref());
    Ok((words, warning))
}

pub(crate) fn help_flag_warning(
    runtime: Runtime,
    words: &[String],
    help_flags: Option<&HashSet<String>>,
) -> Option<String> {
    let listed = help_flags?;
    let reserved = reserved_flags(runtime);
    first_help_warning(
        words,
        command_name(runtime),
        listed,
        reserved.supports_config_keys(),
    )
}

fn first_reserved_conflict(words: &[String], reserved: &ReservedFlags) -> Option<String> {
    let check_config_keys = reserved.supports_config_keys();
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if check_config_keys {
            if matches!(word.as_str(), "-c" | "--config") {
                if let Some(spec) = words.get(i + 1) {
                    if let Some(user_key) = first_overlapping_toml_key(spec, &reserved.config_keys)
                    {
                        return Some(format!("{word} {user_key}"));
                    }
                    i += 2;
                    continue;
                }
            } else if let Some((flag, spec)) = word
                .strip_prefix("--config=")
                .map(|spec| ("--config", spec))
                .or_else(|| word.strip_prefix("-c=").map(|spec| ("-c", spec)))
            {
                if let Some(user_key) = first_overlapping_toml_key(spec, &reserved.config_keys) {
                    return Some(format!("{flag} {user_key}"));
                }
                i += 1;
                continue;
            }
        }
        if word.starts_with('-') {
            let name = flag_name(word);
            if reserved.flags.contains(name) {
                return Some(name.to_owned());
            }
        }
        i += 1;
    }
    None
}

fn first_help_warning(
    words: &[String],
    command: &str,
    listed: &HashSet<String>,
    check_config_keys: bool,
) -> Option<String> {
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if check_config_keys && matches!(word.as_str(), "-c" | "--config") {
            if !listed.contains(word.as_str()) {
                return Some(format!("{command} --help doesn't list {word}."));
            }
            if words.get(i + 1).is_some() {
                i += 2;
                continue;
            }
        } else if check_config_keys && (word.starts_with("-c=") || word.starts_with("--config=")) {
            let name = flag_name(word);
            if !listed.contains(name) {
                return Some(format!("{command} --help doesn't list {name}."));
            }
            i += 1;
            continue;
        } else if word.starts_with('-') {
            let name = flag_name(word);
            if !listed.contains(name) {
                return Some(format!("{command} --help doesn't list {name}."));
            }
        }
        i += 1;
    }
    None
}

fn first_overlapping_toml_key(spec: &str, reserved_keys: &HashSet<String>) -> Option<String> {
    extract_toml_key_paths(spec).into_iter().find(|user_key| {
        reserved_keys
            .iter()
            .any(|res| toml_keys_overlap(user_key, res))
    })
}

fn toml_keys_overlap(a: &str, b: &str) -> bool {
    a == b
        || (a.len() > b.len() && a.starts_with(b) && a.as_bytes()[b.len()] == b'.')
        || (b.len() > a.len() && b.starts_with(a) && b.as_bytes()[a.len()] == b'.')
}

fn extract_toml_key_paths(spec: &str) -> Vec<String> {
    if let Ok(doc) = spec.parse::<toml_edit::DocumentMut>() {
        let mut out = Vec::new();
        collect_table_paths(doc.as_table(), None, &mut out);
        if !out.is_empty() {
            return out;
        }
    }
    if let Some((lhs, _)) = spec.split_once('=') {
        let synthetic = format!("{lhs} = true");
        if let Ok(doc) = synthetic.parse::<toml_edit::DocumentMut>() {
            let mut out = Vec::new();
            collect_table_paths(doc.as_table(), None, &mut out);
            if !out.is_empty() {
                return out;
            }
        }
        let trimmed = lhs.trim();
        if !trimmed.is_empty() {
            return vec![trimmed.to_owned()];
        }
    }
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        Vec::new()
    } else {
        vec![trimmed.to_owned()]
    }
}

fn collect_table_paths(table: &toml_edit::Table, prefix: Option<&str>, out: &mut Vec<String>) {
    if table.is_empty() {
        if let Some(prefix) = prefix {
            out.push(prefix.to_owned());
        }
        return;
    }
    for (key, item) in table.iter() {
        let path = join_toml_path(prefix, key);
        collect_item_paths(item, &path, out);
    }
}

fn collect_inline_table_paths(table: &toml_edit::InlineTable, prefix: &str, out: &mut Vec<String>) {
    if table.is_empty() {
        out.push(prefix.to_owned());
        return;
    }
    for (key, value) in table.iter() {
        let path = join_toml_path(Some(prefix), key);
        match value {
            toml_edit::Value::InlineTable(inner) => {
                collect_inline_table_paths(inner, &path, out);
            }
            _ => out.push(path),
        }
    }
}

fn collect_item_paths(item: &toml_edit::Item, path: &str, out: &mut Vec<String>) {
    match item {
        toml_edit::Item::Table(table) => collect_table_paths(table, Some(path), out),
        toml_edit::Item::Value(toml_edit::Value::InlineTable(table)) => {
            collect_inline_table_paths(table, path, out);
        }
        toml_edit::Item::None => {}
        _ => out.push(path.to_owned()),
    }
}

fn join_toml_path(prefix: Option<&str>, key: &str) -> String {
    let segment = toml_edit::Key::new(key).to_string();
    match prefix {
        Some(prefix) => format!("{prefix}.{segment}"),
        None => segment,
    }
}

pub(crate) fn parse_help_flags(output: &[u8]) -> Option<HashSet<String>> {
    let text = strip_ansi(&String::from_utf8_lossy(output));
    let mut flags = HashSet::new();

    for line in text.lines() {
        let trimmed = line.trim_start();
        let (opt_col, rest) = if trimmed.starts_with('-') {
            split_option_column(trimmed)
        } else {
            ("", trimmed)
        };
        if !opt_col.is_empty() {
            collect_help_flags_from_slice(opt_col, true, &mut flags);
        }
        if !rest.is_empty() {
            collect_help_flags_from_slice(rest, false, &mut flags);
        }
    }

    (!flags.is_empty()).then_some(flags)
}

fn split_option_column(trimmed: &str) -> (&str, &str) {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\t' {
            return (&trimmed[..i], &trimmed[i + 1..]);
        }
        if bytes[i] == b' ' && bytes.get(i + 1) == Some(&b' ') {
            return (&trimmed[..i], &trimmed[i..]);
        }
        i += 1;
    }
    (trimmed, "")
}

fn collect_help_flags_from_slice(
    slice: &str,
    mut allow_multi_short: bool,
    out: &mut HashSet<String>,
) {
    for raw in slice.split_whitespace() {
        for piece in raw.split(',') {
            let token = strip_outer_delimiters(piece);
            if token.starts_with('-') {
                if token.contains('/') {
                    for part in token.split('/') {
                        let part = strip_outer_delimiters(part);
                        if part.starts_with('-') {
                            record_help_flag(part, allow_multi_short, out);
                        }
                    }
                } else {
                    record_help_flag(token, allow_multi_short, out);
                }
            } else if allow_multi_short
                && !token.is_empty()
                && !piece.starts_with('<')
                && !piece.starts_with('[')
                && !piece.starts_with('(')
                && token != "|"
                && token.chars().any(|c| c.is_ascii_lowercase())
            {
                // Stop accepting multi-letter single-dash options once prose begins.
                allow_multi_short = false;
            }
        }
    }
}

fn strip_outer_delimiters(mut token: &str) -> &str {
    loop {
        let before = token;
        token = token.trim_end_matches([',', ';', ':', '.']);
        if let Some(inner) = token.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if !inner.contains('[') {
                token = inner;
            }
        }
        if let Some(inner) = token.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            if !inner.contains('(') {
                token = inner;
            }
        }
        token = token.trim_start_matches(['(', '[']);
        if !token.contains('[') {
            token = token.trim_end_matches(']');
        }
        if !token.contains('(') {
            token = token.trim_end_matches(')');
        }
        if token == before {
            break;
        }
    }
    token
}

fn record_help_flag(token: &str, allow_multi_short: bool, out: &mut HashSet<String>) {
    let base = token
        .split_once('=')
        .map_or(token, |(name, _)| name)
        .trim_end_matches('[');

    // Commander/yargs `--[no-]foo` style.
    if let Some(rest) = base.strip_prefix("--[no-]") {
        let positive = format!("--{rest}");
        let negative = format!("--no-{rest}");
        validate_and_insert(&positive, allow_multi_short, out);
        validate_and_insert(&negative, allow_multi_short, out);
        return;
    }

    // Optional suffix such as `pi`'s `--system-prompt[-file]`.
    if let Some((prefix, bracket)) = base.split_once('[') {
        if let Some(inner) = bracket.strip_suffix(']') {
            validate_and_insert(prefix, allow_multi_short, out);
            if inner.starts_with('-') && !inner.contains('=') {
                let expanded = format!("{prefix}{inner}");
                validate_and_insert(&expanded, allow_multi_short, out);
            }
            return;
        }
        validate_and_insert(prefix, allow_multi_short, out);
        return;
    }

    validate_and_insert(base, allow_multi_short, out);
}

fn validate_and_insert(candidate: &str, allow_multi_short: bool, out: &mut HashSet<String>) {
    if is_valid_help_flag(candidate, allow_multi_short) {
        out.insert(candidate.to_owned());
    }
}

fn is_valid_help_flag(candidate: &str, allow_multi_short: bool) -> bool {
    if let Some(rest) = candidate.strip_prefix("--") {
        let mut chars = rest.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    } else if let Some(rest) = candidate.strip_prefix('-') {
        let mut chars = rest.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() {
            return false;
        }
        match chars.next() {
            None => true,
            Some(_) if allow_multi_short => rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
            Some(_) => false,
        }
    } else {
        false
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\x07' || (next == '\x1b' && chars.peek() == Some(&'\\')) {
                        if next == '\x1b' {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_word_parsing_matches_spec_rules() {
        assert_eq!(parse_custom_flags("").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_custom_flags("   \t\n\r\n  ").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_custom_flags(
                r#"--search -c model_verbosity=high --add-dir "/tmp/runner qa/extra dir""#
            )
            .unwrap(),
            vec![
                "--search",
                "-c",
                "model_verbosity=high",
                "--add-dir",
                "/tmp/runner qa/extra dir",
            ]
        );
        assert_eq!(
            parse_custom_flags("--flag 'single quoted value' --escaped foo\\ bar").unwrap(),
            vec!["--flag", "single quoted value", "--escaped", "foo bar"]
        );
        assert_eq!(
            parse_custom_flags("--first\n--second\r\n--third").unwrap(),
            vec!["--first", "--second", "--third"]
        );
        // Shell expansions are preserved literally because Runner passes argv directly.
        assert_eq!(
            parse_custom_flags("--add-dir ~/repo --env-val $HOME").unwrap(),
            vec!["--add-dir", "~/repo", "--env-val", "$HOME"]
        );
        for bad in ["--add-dir \"unclosed", "--add-dir 'unclosed", "--search \\"] {
            let err = parse_custom_flags(bad).unwrap_err();
            assert_eq!(err.code, "unclosed_quote");
            assert_eq!(err.message, "A quote isn't closed.");
        }
    }

    #[test]
    fn every_composed_flag_across_all_runtimes_is_reserved() {
        let scratch = tempfile::tempdir().unwrap();
        for runtime in Runtime::ALL {
            if runtime == Runtime::Shell {
                continue;
            }
            let adapter = crate::runtimes::adapter(runtime);
            if let Some(hooks) = adapter.status_hooks() {
                hooks.install(scratch.path());
            }
            let reserved = reserved_flags(runtime);
            let prior = sample_prior_key(runtime);

            for mission in [false, true] {
                for bypass in [false, true] {
                    for status_hooks in [false, true] {
                        for remote_plugin_catalog in [false, true] {
                            let options = RuntimeLaunchOptions {
                                bypass,
                                status_hooks,
                                remote_plugin_catalog,
                                custom_flags: String::new(),
                            };
                            for prior_key in [None, Some(prior)] {
                                let (args, _) =
                                    crate::session::manager::spawn_spec::compose_preview_args(
                                        runtime,
                                        mission,
                                        &options,
                                        scratch.path(),
                                        Default::default(),
                                        crate::session::manager::spawn_spec::PreviewArgsOverrides {
                                            prior_key,
                                            codex_speed: Some(CodexSpeed::Fast),
                                            system_prompt: Some("<system prompt>"),
                                            ..Default::default()
                                        },
                                    );
                                check_composed_flags_are_reserved(runtime, &reserved, &args);
                            }
                        }
                    }
                }
            }

            let perms = adapter.permissions();
            for mode in perms
                .offered
                .iter()
                .copied()
                .chain([PermissionMode::Bypass])
            {
                check_composed_flags_are_reserved(runtime, &reserved, &perms.mode_args(mode));
            }
            check_composed_flags_are_reserved(runtime, &reserved, &perms.mission_args());
            check_composed_flags_are_reserved(
                runtime,
                &reserved,
                &adapter.resume_plan(Some(prior)).args,
            );
            if let Some(ForkPlan::Direct(plan)) = adapter.fork_plan(prior, "fork") {
                check_composed_flags_are_reserved(runtime, &reserved, &plan.args);
            }

            if let Some(catalog) = adapter.catalog() {
                let catalog_efforts: Vec<&str> = catalog
                    .efforts
                    .iter()
                    .map(|opt| opt.value.as_str())
                    .filter(|v| !v.is_empty())
                    .collect();
                for model in catalog.models.iter().filter(|m| !m.value.is_empty()) {
                    let efforts: Vec<&str> = match &model.supported_efforts {
                        Some(supported) => supported
                            .iter()
                            .map(String::as_str)
                            .filter(|v| !v.is_empty())
                            .collect(),
                        None => catalog_efforts.clone(),
                    };
                    for effort in efforts {
                        let (args, _) = crate::session::manager::spawn_spec::compose_preview_args(
                            runtime,
                            false,
                            &RuntimeLaunchOptions::default(),
                            scratch.path(),
                            Default::default(),
                            crate::session::manager::spawn_spec::PreviewArgsOverrides {
                                model: Some(model.value.as_str()),
                                effort: Some(effort),
                                codex_speed: Some(CodexSpeed::Fast),
                                system_prompt: Some("<system prompt>"),
                                ..Default::default()
                            },
                        );
                        check_composed_flags_are_reserved(runtime, &reserved, &args);
                    }
                }
            }

            if runtime == Runtime::Grok {
                let grok_home = scratch.path().join("grok-independent-test");
                std::fs::create_dir_all(&grok_home).unwrap();
                std::fs::write(
                    grok_home.join("models_cache.json"),
                    r#"{"models":{"grok-test-model":{"info":{"reasoning_efforts":[{"id":"xhigh"}]}}}}"#,
                )
                .unwrap();
                let mut env = BTreeMap::new();
                env.insert("GROK_HOME".into(), grok_home.to_string_lossy().into_owned());
                let (args, _) = crate::session::manager::spawn_spec::compose_preview_args(
                    runtime,
                    false,
                    &RuntimeLaunchOptions::default(),
                    scratch.path(),
                    env,
                    crate::session::manager::spawn_spec::PreviewArgsOverrides {
                        model: Some("grok-test-model"),
                        effort: Some("xhigh"),
                        ..Default::default()
                    },
                );
                assert!(args.iter().any(|arg| arg == "--reasoning-effort"));
                check_composed_flags_are_reserved(runtime, &reserved, &args);
            }
        }
    }

    #[test]
    fn conditional_effort_flags_are_reserved_and_refused() {
        for (input, expected) in [
            ("--effort high", "Runner already sets --effort."),
            ("--effort=medium", "Runner already sets --effort."),
        ] {
            let words = parse_custom_flags(input).unwrap();
            let err = check_reserved_flags(Runtime::Antigravity, &words).unwrap_err();
            assert_eq!(err.message, expected, "{input}");
        }

        for (input, expected) in [
            (
                "--reasoning-effort high",
                "Runner already sets --reasoning-effort.",
            ),
            (
                "--reasoning-effort=xhigh",
                "Runner already sets --reasoning-effort.",
            ),
        ] {
            let words = parse_custom_flags(input).unwrap();
            let err = check_reserved_flags(Runtime::Grok, &words).unwrap_err();
            assert_eq!(err.message, expected, "{input}");
        }

        for (runtime, input, expected) in [
            (
                Runtime::ClaudeCode,
                "--effort high",
                "Runner already sets --effort.",
            ),
            (
                Runtime::Copilot,
                "--effort high",
                "Runner already sets --effort.",
            ),
            (
                Runtime::Pi,
                "--thinking high",
                "Runner already sets --thinking.",
            ),
            (
                Runtime::Codex,
                "-c model_reasoning_effort=high",
                "Runner already sets -c model_reasoning_effort.",
            ),
            (
                Runtime::Trae,
                "-c model_reasoning_effort=high",
                "Runner already sets -c model_reasoning_effort.",
            ),
        ] {
            let words = parse_custom_flags(input).unwrap();
            let err = check_reserved_flags(runtime, &words).unwrap_err();
            assert_eq!(err.message, expected, "{runtime}: {input}");
        }
    }

    fn check_composed_flags_are_reserved(
        runtime: Runtime,
        reserved: &ReservedFlags,
        args: &[String],
    ) {
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if reserved.supports_config_keys() && matches!(arg.as_str(), "-c" | "--config") {
                if let Some(spec) = args.get(i + 1) {
                    let err = first_reserved_conflict(&[arg.clone(), spec.clone()], reserved);
                    assert!(
                        err.is_some(),
                        "{runtime}: composed config arg `{arg} {spec}` is not reserved"
                    );
                    i += 2;
                    continue;
                }
            }
            if arg.starts_with('-') {
                let name = flag_name(arg);
                if name != "--add-dir" {
                    assert!(
                        reserved.flags.contains(name),
                        "{runtime}: composed flag `{name}` (from `{arg}`) is not reserved"
                    );
                }
            }
            i += 1;
        }
    }

    #[test]
    fn codex_reserved_checks_allow_add_dir_and_unreserved_config_keys() {
        for ok in [
            "--search -c model_verbosity=high --add-dir \"/tmp/a b\"",
            "-c 'features={web_search=true}'",
            "-m gpt-5",
            "--search positional-word",
        ] {
            let words = parse_custom_flags(ok).unwrap();
            assert_eq!(check_reserved_flags(Runtime::Codex, &words), Ok(()), "{ok}");
        }

        for (bad, expected) in [
            ("--model gpt-5", "Runner already sets --model."),
            ("--model=gpt-5", "Runner already sets --model."),
            (
                "-c features.remote_plugin=true",
                "Runner already sets -c features.remote_plugin.",
            ),
            (
                "--config features.remote_plugin=false",
                "Runner already sets --config features.remote_plugin.",
            ),
            (
                "-c features.remote_plugin=bareword",
                "Runner already sets -c features.remote_plugin.",
            ),
            (
                "-c 'features={remote_plugin=true}'",
                "Runner already sets -c features.remote_plugin.",
            ),
            ("-c 'hooks={}'", "Runner already sets -c hooks."),
            (
                "-c mcp_servers.runner_hooks.enabled=false",
                "Runner already sets -c mcp_servers.runner_hooks.enabled.",
            ),
            (
                "--ask-for-approval never",
                "Runner already sets --ask-for-approval.",
            ),
            (
                "--sandbox workspace-write",
                "Runner already sets --sandbox.",
            ),
        ] {
            let words = parse_custom_flags(bad).unwrap();
            let err = check_reserved_flags(Runtime::Codex, &words).unwrap_err();
            assert_eq!(err.message, expected, "{bad}");
        }
    }

    #[test]
    fn reserved_permission_flags_are_refused_for_every_runtime() {
        for runtime in Runtime::ALL {
            if runtime == Runtime::Shell {
                continue;
            }
            let adapter = crate::runtimes::adapter(runtime);
            let perms = adapter.permissions();
            for &(flag, _) in perms.strip_flags {
                if flag == "--add-dir" {
                    continue;
                }
                let words = parse_custom_flags(flag).unwrap();
                let err = check_reserved_flags(runtime, &words).unwrap_err();
                assert_eq!(
                    err.message,
                    format!("Runner already sets {flag}."),
                    "{runtime}"
                );
            }
            for flag in perms
                .mission_args()
                .into_iter()
                .chain(
                    perms
                        .offered
                        .iter()
                        .copied()
                        .chain([PermissionMode::Bypass])
                        .flat_map(|mode| perms.mode_args(mode)),
                )
                .filter(|arg| arg.starts_with('-'))
            {
                let words = parse_custom_flags(&flag).unwrap();
                let err = check_reserved_flags(runtime, &words).unwrap_err();
                let expected_flag = flag.split_once('=').map_or(flag.as_str(), |(name, _)| name);
                assert_eq!(
                    err.message,
                    format!("Runner already sets {expected_flag}."),
                    "{runtime}"
                );
            }
        }
        // Resume / fork flags outside `permissions()` are also refused.
        for (runtime, flag) in [
            (Runtime::OpenCode, "--session"),
            (Runtime::Grok, "-s"),
            (Runtime::Grok, "--resume"),
            (Runtime::Grok, "--fork-session"),
            (Runtime::Pi, "--fork"),
            (Runtime::Pi, "--session-id"),
        ] {
            let words = parse_custom_flags(flag).unwrap();
            let err = check_reserved_flags(runtime, &words).unwrap_err();
            assert_eq!(
                err.message,
                format!("Runner already sets {flag}."),
                "{runtime}"
            );
        }
    }

    #[test]
    fn help_parser_extracts_flags_across_agent_formats() {
        let codex = parse_help_flags(
            b"\x1b[1mUsage:\x1b[0m codex [OPTIONS]\n  -c, --config <key=value>  Override config\n      --search              Enable web search\n      --enable <FEATURE>    Enable feature\n      --add-dir <DIR>       Extra writable dir\n",
        )
        .unwrap();
        for f in ["-c", "--config", "--search", "--enable", "--add-dir"] {
            assert!(codex.contains(f), "missing {f} in codex: {codex:?}");
        }

        let claude = parse_help_flags(
            b"Options:\n  --allowedTools, --allowed-tools <tools...>  Allowed tools\n  -d, --debug [filter]                         Debug mode\n  -p, --print                                  Print response\n",
        )
        .unwrap();
        for f in [
            "--allowedTools",
            "--allowed-tools",
            "-d",
            "--debug",
            "-p",
            "--print",
        ] {
            assert!(claude.contains(f), "missing {f}");
        }

        let agy = parse_help_flags(
            b"Flags:\n  -mode <mode>  Execution mode\n  -v, --version  Show version\n  -h/--help      Show help\n",
        )
        .unwrap();
        for f in ["-mode", "-v", "--version", "-h", "--help"] {
            assert!(agy.contains(f), "missing {f}");
        }

        let pi = parse_help_flags(
            b"Options:\n  -nt, --no-tools              Disable tools\n  -nbt                         Disable built-in tools\n  -xt                          Extra terse\n  --system-prompt[-file] <p>   System prompt (+name/-name prefixes)\n",
        )
        .unwrap();
        for f in [
            "-nt",
            "--no-tools",
            "-nbt",
            "-xt",
            "--system-prompt",
            "--system-prompt-file",
        ] {
            assert!(pi.contains(f), "missing {f}");
        }
        assert!(
            !pi.contains("-name"),
            "should not match +name/-name in prose"
        );

        let copilot =
            parse_help_flags(b"  --allow-all   Allow all tools\n  -p, --prompt <text>  Prompt\n")
                .unwrap();
        assert!(
            copilot.contains("--allow-all")
                && copilot.contains("-p")
                && copilot.contains("--prompt")
        );

        let cursor = parse_help_flags(b"  --yolo   Auto approve\n  -f, --force  Force\n").unwrap();
        assert!(cursor.contains("--yolo") && cursor.contains("-f") && cursor.contains("--force"));

        let grok =
            parse_help_flags(b"  -m, --model <model>  Model\n  --max-tool-rounds <n>  Rounds\n")
                .unwrap();
        assert!(
            grok.contains("-m") && grok.contains("--model") && grok.contains("--max-tool-rounds")
        );

        let opencode =
            parse_help_flags(b"  --session, -s <id>  Session\n  -m, --model <m>  Model\n").unwrap();
        assert!(
            opencode.contains("--session")
                && opencode.contains("-s")
                && opencode.contains("--model")
        );

        let trae =
            parse_help_flags(b"  --yolo  Skip approvals\n  --flag=value  Example\n").unwrap();
        assert!(trae.contains("--yolo") && trae.contains("--flag"));

        assert_eq!(parse_help_flags(b""), None);
        assert_eq!(parse_help_flags(b"No flags listed here."), None);
    }

    #[test]
    fn validation_check_order_and_warning_rules() {
        let help: HashSet<String> = ["-c", "--config", "--search", "--add-dir", "--foo"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        // Check 1 beats 2 and 3.
        let err = validate_save(Runtime::Codex, "--serch --model \"unclosed", || {
            Some(help.clone())
        })
        .unwrap_err();
        assert_eq!(err.message, "A quote isn't closed.");

        // Check 2 beats 3.
        let err = validate_save(Runtime::Codex, "--serch --model gpt-5", || {
            Some(help.clone())
        })
        .unwrap_err();
        assert_eq!(err.message, "Runner already sets --model.");

        // Check 3 warns while saving, matching flag name before `=`.
        let (words, warning) = validate_save(Runtime::Codex, "--serch=yes --foo=bar", || {
            Some(help.clone())
        })
        .unwrap();
        assert_eq!(words, vec!["--serch=yes", "--foo=bar"]);
        assert_eq!(
            warning.as_deref(),
            Some("codex --help doesn't list --serch.")
        );

        // `-c model_verbosity=-1` skips value `-1` and does not warn.
        let (_, warning) = validate_save(Runtime::Codex, "-c model_verbosity=-1 --foo=bar", || {
            Some(help.clone())
        })
        .unwrap();
        assert_eq!(warning, None);

        // Unreadable help (`None`) produces no warning.
        let (_, warning) = validate_save(Runtime::Codex, "--serch", || None).unwrap();
        assert_eq!(warning, None);
    }
}
