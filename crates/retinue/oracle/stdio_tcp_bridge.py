"""Bridge this process's stdin and stdout to a TCP connection, for `interop_pipe.py`.

`python -I stdio_tcp_bridge.py PORT` dials 127.0.0.1:PORT, prints `BRIDGE_PID <pid>` on
stderr, and copies bytes both ways until either side closes, then exits. As a retinue
PipeInterface's child it puts the pipe's HDLC on a stock TCPServerInterface, which frames
the same way.
"""
import os
import socket
import sys
import threading


def main() -> None:
    sock = socket.create_connection(("127.0.0.1", int(sys.argv[1])))
    print(f"BRIDGE_PID {os.getpid()}", file=sys.stderr, flush=True)

    def upstream() -> None:
        while chunk := os.read(0, 4096):
            sock.sendall(chunk)
        os._exit(0)

    threading.Thread(target=upstream, daemon=True).start()
    out = sys.stdout.buffer
    while chunk := sock.recv(4096):
        out.write(chunk)
        out.flush()
    os._exit(0)


if __name__ == "__main__":
    main()
