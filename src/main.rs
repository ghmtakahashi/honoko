use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::{self, Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dialoguer::{Confirm, FuzzySelect, Input, theme::ColorfulTheme};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(version, about = "Run shell commands from a JSON command palette")]
struct Cli {
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[arg(short, long)]
    group: Option<String>,

    #[arg(index = 1, conflicts_with = "group")]
    group_name: Option<String>,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    Init,
    Add {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        executable: Option<String>,
        #[arg(long)]
        args: Vec<String>,
        #[arg(long)]
        shell: Option<String>,
        #[arg(long, default_value_t = false)]
        confirm: bool,
        #[arg(long, default_value_t = true)]
        enabled: bool,
    },
    Remove {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        group: Option<String>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    ungrouped: Vec<StoredCommandEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    groups: BTreeMap<String, Vec<StoredCommandEntry>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredCommandEntry {
    name: String,
    #[serde(flatten)]
    execution: ExecutionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    confirm: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ExecutionConfig {
    Program {
        executable: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    Shell {
        shell: String,
    },
}

#[derive(Clone, Debug)]
struct CommandEntry {
    name: String,
    execution: ExecutionConfig,
    group: Option<String>,
    description: Option<String>,
    confirm: bool,
    enabled: bool,
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => {
            process::exit(code);
        }
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let config_path = resolve_config_path(cli.config)?;

    match cli.command {
        Some(CliCommand::Init) => init_config(&config_path, true),
        Some(CliCommand::Add {
            name,
            group,
            description,
            executable,
            args,
            shell,
            confirm,
            enabled,
        }) => add_command(
            &config_path,
            name,
            group,
            description,
            executable,
            args,
            shell,
            confirm,
            enabled,
        ),
        Some(CliCommand::Remove { name, group }) => remove_command(&config_path, name, group),
        None => run_selector(&config_path, cli.group.or(cli.group_name)),
    }
}

fn resolve_config_path(config: Option<PathBuf>) -> Result<PathBuf> {
    match config {
        Some(path) => Ok(path),
        None => {
            let home = std::env::var_os("HOME").context("HOME is not set")?;
            Ok(PathBuf::from(home).join(".honoko.json"))
        }
    }
}

fn run_selector(path: &PathBuf, group: Option<String>) -> Result<i32> {
    ensure_config_for_selector(path)?;
    let config = load_config(path)?;
    let commands = enabled_commands(config, group.as_deref());

    if commands.is_empty() {
        match group {
            Some(group) => bail!(
                "no enabled commands found in group `{group}` in {}",
                path.display()
            ),
            None => bail!("no enabled commands found in {}", path.display()),
        }
    }

    let theme = ColorfulTheme::default();
    let labels = build_labels(&commands);

    let selected_index = FuzzySelect::with_theme(&theme)
        .with_prompt("Select a command")
        .items(&labels)
        .default(0)
        .interact()
        .context("failed to read selection")?;

    let selected_command = &commands[selected_index];

    if selected_command.confirm {
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt(format!("Run `{}`?", selected_command.name))
            .default(false)
            .interact()
            .context("failed to read confirmation")?;

        if !confirmed {
            println!("Canceled.");
            return Ok(0);
        }
    }

    execute_command(selected_command)
}

fn ensure_config_for_selector(path: &PathBuf) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    let theme = ColorfulTheme::default();
    println!("Config not found: {}", path.display());
    let create_now = Confirm::with_theme(&theme)
        .with_prompt("Create a starter config now?")
        .default(true)
        .interact()
        .context("failed to read config creation confirmation")?;

    if !create_now {
        bail!(
            "create {} or run `honoko init` to generate a starter config",
            path.display()
        );
    }

    init_config(path, false)?;
    Ok(())
}

fn load_config(path: &PathBuf) -> Result<Config> {
    ensure_secure_permissions(path)?;
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let config: Config = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config)
}

fn load_or_default_config(path: &PathBuf) -> Result<Config> {
    if path.exists() {
        load_config(path)
    } else {
        Ok(Config::default())
    }
}

fn save_config(path: &PathBuf, config: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    enforce_secure_permissions(path)?;
    Ok(())
}

fn init_config(path: &PathBuf, fail_if_exists: bool) -> Result<i32> {
    if path.exists() {
        if fail_if_exists {
            bail!("config already exists at {}", path.display());
        }
        return Ok(0);
    }

    let config = starter_config();
    save_config(path, &config)?;
    println!("Created starter config at {}", path.display());
    Ok(0)
}

fn starter_config() -> Config {
    let mut groups = BTreeMap::new();
    groups.insert(
        "rust".to_string(),
        vec![
            StoredCommandEntry {
                name: "Build".to_string(),
                execution: ExecutionConfig::Program {
                    executable: "cargo".to_string(),
                    args: vec!["build".to_string()],
                },
                description: Some("Compile the project".to_string()),
                confirm: false,
                enabled: true,
            },
            StoredCommandEntry {
                name: "Test".to_string(),
                execution: ExecutionConfig::Program {
                    executable: "cargo".to_string(),
                    args: vec!["test".to_string()],
                },
                description: Some("Run the test suite".to_string()),
                confirm: false,
                enabled: true,
            },
        ],
    );
    groups.insert(
        "ops".to_string(),
        vec![StoredCommandEntry {
            name: "Deploy".to_string(),
            execution: ExecutionConfig::Shell {
                shell: "./deploy.sh\n./notify.sh".to_string(),
            },
            description: Some("Run the deploy script".to_string()),
            confirm: true,
            enabled: true,
        }],
    );

    Config {
        ungrouped: vec![StoredCommandEntry {
            name: "Open Logs".to_string(),
            execution: ExecutionConfig::Shell {
                shell: "tail -f /tmp/app.log".to_string(),
            },
            description: Some("Tail the shared log file".to_string()),
            confirm: false,
            enabled: true,
        }],
        groups,
    }
}

fn enabled_commands(config: Config, group: Option<&str>) -> Vec<CommandEntry> {
    flatten_commands(config)
        .into_iter()
        .filter(|command| command.enabled)
        .filter(|command| match group {
            Some(group) => command.group.as_deref() == Some(group),
            None => true,
        })
        .collect()
}

fn flatten_commands(config: Config) -> Vec<CommandEntry> {
    let mut commands: Vec<CommandEntry> = config
        .ungrouped
        .into_iter()
        .map(|entry| CommandEntry {
            name: entry.name,
            execution: entry.execution,
            group: None,
            description: entry.description,
            confirm: entry.confirm,
            enabled: entry.enabled,
        })
        .collect();

    for (group, entries) in config.groups {
        commands.extend(entries.into_iter().map(|entry| CommandEntry {
            name: entry.name,
            execution: entry.execution,
            group: Some(group.clone()),
            description: entry.description,
            confirm: entry.confirm,
            enabled: entry.enabled,
        }));
    }

    commands
}

fn command_exists_in_group(config: &Config, name: &str, group: Option<&str>) -> bool {
    match group {
        Some(group) => config
            .groups
            .get(group)
            .map(|entries| entries.iter().any(|entry| entry.name == name))
            .unwrap_or(false),
        None => config.ungrouped.iter().any(|entry| entry.name == name),
    }
}

fn build_labels(commands: &[CommandEntry]) -> Vec<String> {
    commands
        .iter()
        .map(|command| {
            let mut label = command.name.clone();
            if let Some(group) = &command.group {
                label.push_str(&format!(" [{}]", group));
            }
            if let Some(description) = &command.description {
                label.push_str(&format!(" - {}", description));
            }
            label
        })
        .collect()
}

fn add_command(
    path: &PathBuf,
    name: Option<String>,
    group: Option<String>,
    description: Option<String>,
    executable: Option<String>,
    args: Vec<String>,
    shell: Option<String>,
    confirm: bool,
    enabled: bool,
) -> Result<i32> {
    let theme = ColorfulTheme::default();
    let mut config = load_or_default_config(path)?;
    let name = prompt_required(name, "Command name", &theme)?;
    let group = prompt_optional(group, "Group (optional)", &theme)?;
    let description = prompt_optional(description, "Description (optional)", &theme)?;
    let execution = prompt_execution(executable, args, shell, &theme)?;

    if command_exists_in_group(&config, &name, group.as_deref()) {
        match &group {
            Some(group) => bail!(
                "command `{name}` already exists in group `{group}` in {}",
                path.display()
            ),
            None => bail!(
                "command `{name}` already exists in the ungrouped section in {}",
                path.display()
            ),
        }
    }

    let entry = StoredCommandEntry {
        name: name.clone(),
        execution,
        description,
        confirm,
        enabled,
    };

    match group {
        Some(group) => config.groups.entry(group).or_default().push(entry),
        None => config.ungrouped.push(entry),
    }

    save_config(path, &config)?;
    println!("Added `{name}` to {}", path.display());
    Ok(0)
}

fn remove_command(path: &PathBuf, name: Option<String>, group: Option<String>) -> Result<i32> {
    let theme = ColorfulTheme::default();
    let mut config = load_config(path)?;
    let commands = flatten_commands(config.clone());

    if commands.is_empty() {
        bail!("no commands found in {}", path.display());
    }

    let selected_command = match name {
        Some(name) => select_command_for_removal(&commands, &name, group.as_deref(), &theme)?,
        None => {
            let labels = build_labels(&commands);
            let selected_index = FuzzySelect::with_theme(&theme)
                .with_prompt("Select a command to remove")
                .items(&labels)
                .default(0)
                .interact()
                .context("failed to read removal selection")?;
            commands[selected_index].clone()
        }
    };

    let confirmed = Confirm::with_theme(&theme)
        .with_prompt(format!("Remove `{}`?", selected_command.name))
        .default(false)
        .interact()
        .context("failed to read removal confirmation")?;

    if !confirmed {
        println!("Canceled.");
        return Ok(0);
    }

    remove_stored_command(
        &mut config,
        &selected_command.name,
        selected_command.group.as_deref(),
    )
    .with_context(|| format!("command `{}` was not found", selected_command.name))?;
    save_config(path, &config)?;
    println!(
        "Removed `{}` from {}",
        selected_command.name,
        path.display()
    );
    Ok(0)
}

fn select_command_for_removal(
    commands: &[CommandEntry],
    name: &str,
    group: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<CommandEntry> {
    let matches: Vec<CommandEntry> = commands
        .iter()
        .filter(|entry| entry.name == name)
        .filter(|entry| match group {
            Some(group) => entry.group.as_deref() == Some(group),
            None => true,
        })
        .cloned()
        .collect();

    match matches.len() {
        0 => match group {
            Some(group) => bail!("command `{name}` was not found in group `{group}`"),
            None => bail!("command `{name}` was not found"),
        },
        1 => Ok(matches.into_iter().next().expect("single match exists")),
        _ => {
            let labels = build_labels(&matches);
            let selected_index = FuzzySelect::with_theme(theme)
                .with_prompt("Multiple commands matched; select one to remove")
                .items(&labels)
                .default(0)
                .interact()
                .context("failed to read removal selection")?;
            Ok(matches[selected_index].clone())
        }
    }
}

fn remove_stored_command(config: &mut Config, name: &str, group: Option<&str>) -> Result<()> {
    match group {
        Some(group) => {
            let entries = config
                .groups
                .get_mut(group)
                .with_context(|| format!("group `{group}` was not found"))?;
            let index = entries
                .iter()
                .position(|entry| entry.name == name)
                .with_context(|| format!("command `{name}` was not found"))?;
            entries.remove(index);
            if entries.is_empty() {
                config.groups.remove(group);
            }
        }
        None => {
            let index = config
                .ungrouped
                .iter()
                .position(|entry| entry.name == name)
                .with_context(|| format!("command `{name}` was not found"))?;
            config.ungrouped.remove(index);
        }
    }

    Ok(())
}

fn prompt_required(value: Option<String>, prompt: &str, theme: &ColorfulTheme) -> Result<String> {
    match value {
        Some(value) => Ok(value),
        None => Input::with_theme(theme)
            .with_prompt(prompt)
            .interact_text()
            .with_context(|| format!("failed to read {prompt}")),
    }
}

fn prompt_optional(
    value: Option<String>,
    prompt: &str,
    theme: &ColorfulTheme,
) -> Result<Option<String>> {
    let input = match value {
        Some(value) => value,
        None => Input::with_theme(theme)
            .allow_empty(true)
            .with_prompt(prompt)
            .interact_text()
            .with_context(|| format!("failed to read {prompt}"))?,
    };

    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn execute_command(command: &CommandEntry) -> Result<i32> {
    println!("Running: {}", command_preview(command));

    let status = match &command.execution {
        ExecutionConfig::Program { executable, args } => Command::new(executable)
            .args(args)
            .status()
            .with_context(|| format!("failed to execute `{}`", command.name))?,
        ExecutionConfig::Shell { shell } => Command::new("sh")
            .arg("-c")
            .arg(shell)
            .status()
            .with_context(|| format!("failed to execute `{}`", command.name))?,
    };

    Ok(status.code().unwrap_or(1))
}

fn command_preview(command: &CommandEntry) -> String {
    match &command.execution {
        ExecutionConfig::Program { executable, args } => {
            if args.is_empty() {
                executable.clone()
            } else {
                format!("{executable} {}", shell_words::join(args))
            }
        }
        ExecutionConfig::Shell { shell } => shell.clone(),
    }
}

fn prompt_execution(
    executable: Option<String>,
    args: Vec<String>,
    shell: Option<String>,
    theme: &ColorfulTheme,
) -> Result<ExecutionConfig> {
    match (executable, shell) {
        (Some(_), Some(_)) => bail!("specify either `--executable` or `--shell`, not both"),
        (Some(executable), None) => Ok(ExecutionConfig::Program { executable, args }),
        (None, Some(shell)) => {
            if !args.is_empty() {
                bail!("`--args` can only be used with `--executable`");
            }
            Ok(ExecutionConfig::Shell { shell })
        }
        (None, None) => {
            let use_shell = Confirm::with_theme(theme)
                .with_prompt("Use shell mode?")
                .default(false)
                .interact()
                .context("failed to read execution mode")?;

            if use_shell {
                let shell = prompt_required(None, "Shell command", theme)?;
                Ok(ExecutionConfig::Shell { shell })
            } else {
                let executable = prompt_required(None, "Executable", theme)?;
                let raw_args = prompt_optional(None, "Arguments (optional)", theme)?;
                let args = match raw_args {
                    Some(raw_args) => {
                        shell_words::split(&raw_args).context("failed to parse arguments input")?
                    }
                    None => Vec::new(),
                };
                Ok(ExecutionConfig::Program { executable, args })
            }
        }
    }
}

fn ensure_secure_permissions(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata =
            fs::metadata(path).with_context(|| format!("failed to read {}", path.display()))?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            bail!(
                "{} has insecure permissions {:o}; restrict it to owner-only access (for example `chmod 600 {}`)",
                path.display(),
                mode,
                path.display()
            );
        }
    }

    Ok(())
}

fn enforce_secure_permissions(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure permissions on {}", path.display()))?;
    }

    Ok(())
}
