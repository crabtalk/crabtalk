# Services

Standalone binaries that run as system services and connect to the crabtalk
daemon as clients. Installed and managed by `crabup`.

| Service | Crate | What it does |
| ------- | ----- | ------------ |
| [Cron](cron) | `crabtalk-cron` | Scheduled skill triggers |
| [Outlook](outlook) | `crabtalk-outlook` | Outlook MCP server (email + calendar) |
| [Search](search) | `crabtalk-search` | Meta-search aggregator |
