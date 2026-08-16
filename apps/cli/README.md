# crabtalk-cli

Management commands for the Crabtalk daemon. The binary is `crabtalk`.

Every subcommand is non-interactive admin over the daemon connection — a
Unix domain socket, or TCP with `--tcp`.

## Usage

```bash
crabtalk agent list          # create, list, delete, rename agents
crabtalk mcp list            # manage an agent's MCP servers
crabtalk pkg                 # skills and MCPs as packages
crabtalk reload              # hot-reload daemon configuration
crabtalk auth                # cloud authentication
```

Chat lives in ACP clients (e.g. [cydonia](https://github.com/crabtalk/cydonia)),
not here.
