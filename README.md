# honoko

`honoko` is a small Rust CLI that loads shell commands from JSON, lets you search them interactively, and runs the selected command.

## Config

By default, `honoko` reads `~/.honoko.json`:

```json
{
  "ungrouped": [
    {
      "name": "Open Logs",
      "description": "Tail the shared log file",
      "shell": "tail -f /tmp/app.log"
    }
  ],
  "groups": {
    "rust": [
      {
        "name": "Build",
        "description": "Compile the project",
        "executable": "cargo",
        "args": ["build"]
      },
      {
        "name": "Test",
        "description": "Run the test suite",
        "executable": "cargo",
        "args": ["test"]
      }
    ],
    "ops": [
      {
        "name": "Deploy",
        "description": "Run the deploy script",
        "shell": "./deploy.sh\n./notify.sh",
        "confirm": true
      }
    ]
  }
}
```

If `~/.honoko.json` does not exist, `honoko` offers to create this starter config the first time you run it.

## Usage

Build a release binary:

```bash
cargo build --release
```

Run the built binary:

```bash
./target/release/honoko
```

If you place the binary on your `PATH`, you can run it directly:

```bash
honoko
```

Create the starter config explicitly:

```bash
honoko init
```

Filter by group:

```bash
honoko rust
```

Or specify a different config file:

```bash
honoko --config ./honoko.json
```

You can still use `honoko -g rust` if you prefer an explicit flag.

## Development

For local development, you can still use `cargo run`:

```bash
cargo run
```

Create the starter config explicitly with Cargo:

```bash
cargo run -- init
```

Run a specific group with Cargo:

```bash
cargo run -- rust
```

Or specify a different config file with Cargo:

```bash
cargo run -- --config ./honoko.json
```

Type to filter commands, move with the arrow keys, and press Enter to run the selected command.

## Manage commands

Add a program-style command with flags:

```bash
honoko add --name Test --group rust --executable cargo --args test --description "Run tests"
```

Add a shell-style command:

```bash
honoko add --name Deploy --group ops --shell "./deploy.sh && ./notify.sh" --description "Run deploy" --confirm
```

Or run `honoko add` and fill the prompts interactively.

Remove a command by name:

```bash
honoko remove --name Deploy --group ops
```

If the same name exists in multiple groups, `honoko` asks which one to remove unless you pass `--group`.

Or run `honoko remove` and choose the command from the list.

## Security

`honoko` rejects config files that are accessible by group or other users on Unix-like systems. Set owner-only permissions before running:

```bash
chmod 600 ~/.honoko.json
```

Prefer `executable` + `args` for normal commands, and use `shell` only when you need shell syntax such as pipes or multi-line scripts.
