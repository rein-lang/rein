# Install a program

This page is the single source for installing a Rein program. Other pages link here.

## Install with git clone

Clone the program repository into the user programs folder. Use one folder for each program:

```sh
git clone <repo-url> ~/.config/rein/programs/<name>
```

- The repository root is the program folder. A program in a subfolder of a repository does not load.
- `<name>` is the program name. Without a `name` field in `rein.json`, the folder name is the program name.
- The user programs folder depends on your environment:
  - If `REIN_HOME` is set, the folder is `$REIN_HOME`.
  - Otherwise, if `XDG_CONFIG_HOME` is set, the folder is `$XDG_CONFIG_HOME/rein/programs/`.
  - Otherwise, the folder is `~/.config/rein/programs/`.

## Program folder

A program folder holds `rein.json`, `main.rein`, or both. A folder with only `main.rein` is a program.

The manifest is JSON. Every field is optional:

```json
{
  "name": "support-ticket",
  "description": "Decide how the support agent handles a customer ticket.",
  "entry": "main.rein"
}
```

| Field | Default |
| --- | --- |
| `name` | The folder name |
| `description` | The first paragraph of the doc comment in the entry file |
| `entry` | `main.rein` |

- A `rein.json` that is not valid JSON is ignored. The defaults apply.
- A folder that starts with `.` is not a program.
- A folder in a scope folder that has neither file is not listed.

## Scopes

Rein finds programs in three scopes:

| Scope | Folder | Precedence |
| --- | --- | --- |
| Project | A `rein/` folder in the working directory or a parent folder | 1, highest |
| User | The user programs folder (see above) | 2 |
| System | `/etc/rein/`. On macOS also `/Library/Application Support/Rein/` | 3, lowest |

- On a name clash, the higher scope wins.
- Inside the project scope, the nearest `rein/` folder wins.
- A program that loses a clash is shadowed. `rein list` marks it `(shadowed by <scope>)`. The name runs the winning program.
- `REIN_SYSTEM_DIRS` replaces the system folders. It is a colon-separated list.

## Confirm the install

Run these commands:

```sh
rein list
rein show <name>
```

- `rein list` shows each visible program: name, scope, path, description, and first question. Add `--json` for JSON output.
- `rein show <name>` prints the manifest and every `.rein` file. The entry file comes first.
- A program you installed in the user programs folder has the scope `user`.
- User programs appear from any directory. Project programs appear only under their project.

## Run the program

```sh
rein run <name>
```

`rein run <name>` starts the program from any directory. Answer each question on stdin. An agent starts the same program by name with `rein_start`.

## Update

Pull the newest files into the program folder:

```sh
git -C ~/.config/rein/programs/<name> pull
```

Each `rein` command reads the files again. A live run in `rein serve --mcp` keeps the files it loaded at start.

## Remove

Delete the program folder:

```sh
rm -r ~/.config/rein/programs/<name>
```

Run `rein list` to confirm that the program is gone.

## Agent hook

`rein install claude-code` is a different topic. It writes the agent hook, not a program. See [spec section 12](docs/spec.md#12-agent-integration).
