import argparse
import asyncio
import json
from pathlib import Path

import asyncssh


class PasswordServer(asyncssh.SSHServer):
    def __init__(self, username: str, password: str) -> None:
        self.username = username
        self.password = password

    def begin_auth(self, username: str) -> bool:
        return True

    def password_auth_supported(self) -> bool:
        return True

    def validate_password(self, username: str, password: str) -> bool:
        return username == self.username and password == self.password


async def serve(args: argparse.Namespace) -> None:
    root = Path(args.root).resolve()
    root.mkdir(parents=True, exist_ok=True)
    # Apache MINA SSHD on Android has no Ed25519 provider by default.
    host_key = asyncssh.generate_private_key("ssh-rsa", key_size=3072)
    server = await asyncssh.create_server(
        lambda: PasswordServer(args.username, args.password),
        args.host,
        args.port,
        server_host_keys=[host_key],
        sftp_factory=lambda channel: asyncssh.SFTPServer(channel, chroot=str(root)),
    )
    sockets = server.sockets or []
    port = sockets[0].getsockname()[1] if sockets else args.port
    print(json.dumps({"host": args.host, "port": port, "root": str(root)}), flush=True)
    await server.wait_closed()


def main() -> None:
    parser = argparse.ArgumentParser(description="Temporary LAN Chat SFTP benchmark server")
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=22022)
    parser.add_argument("--root", required=True)
    parser.add_argument("--username", default="lanchat-benchmark")
    parser.add_argument("--password", required=True)
    args = parser.parse_args()
    try:
        asyncio.run(serve(args))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
