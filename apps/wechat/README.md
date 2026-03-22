# crabtalk-wechat

WeChat bot gateway for [Crabtalk](https://github.com/crabtalk/crabtalk).

Connects WeChat users to Crabtalk agents via the daemon's Unix socket. Uses the
ilink bot HTTP API for message polling and delivery. Authenticates via QR code
scan on first run.

## Usage

```bash
crabtalk-wechat gateway-wechat start
```

## License

GPL-3.0
