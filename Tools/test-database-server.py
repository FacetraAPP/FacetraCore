"""Loopback protocol fixtures for testing the real Rust database drivers.

No database installation, external network, or user credentials are needed.
The fixtures implement trust/native-password handshakes and prepared queries;
they echo the bound string as a row so tests detect accidental interpolation.
"""

import socket
import struct
import sys

stall = "stall" in sys.argv[2:]
legacy = "legacy" in sys.argv[2:]
typed = "types" in sys.argv[2:]
empty = "empty" in sys.argv[2:]
all_null = "null" in sys.argv[2:]


def receive(connection, length):
    result = b""
    while len(result) < length:
        chunk = connection.recv(length - len(result))
        if not chunk:
            raise EOFError
        result += chunk
    return result


def postgres(connection):
    size = struct.unpack(">I", receive(connection, 4))[0]
    receive(connection, size - 4)

    def send(kind, payload=b""):
        connection.sendall(kind + struct.pack(">I", len(payload) + 4) + payload)

    send(b"R", struct.pack(">I", 0))
    send(b"S", b"server_version\x0016.0\x00")
    send(b"S", b"client_encoding\x00UTF8\x00")
    send(b"K", struct.pack(">II", 1, 2))
    send(b"Z", b"I")
    fields = [(b"label", 25, -1), (b"amount", 20, 8)]
    values = None
    if typed:
        fields = [
            (b"boolean", 16, 1),
            (b"i16", 21, 2),
            (b"i32", 23, 4),
            (b"i64", 20, 8),
            (b"f32", 700, 4),
            (b"f64", 701, 8),
            (b"decimal", 1700, -1),
            (b"binary", 17, -1),
            (b"date", 1082, 4),
            (b"time", 1083, 8),
            (b"timestamp", 1114, 8),
            (b"utc", 1184, 8),
            (b"json", 114, -1),
            (b"jsonb", 3802, -1),
            (b"uuid", 2950, 16),
            (b"money", 790, 8),
            (b"missing", 25, -1),
        ]
        values = [
            b"\x01",
            struct.pack(">h", -12),
            struct.pack(">i", 123456),
            struct.pack(">q", 9223372036854775807),
            struct.pack(">f", 1.5),
            struct.pack(">d", 2.5),
            struct.pack(">hhhhhh", 2, 0, 0, 2, 12, 5000),
            b"\x00\xff",
            struct.pack(">i", 0),
            struct.pack(">q", 3661000002),
            struct.pack(">q", 1000002),
            struct.pack(">q", 1000002),
            b'{"x":1}',
            b'\x01{"x":2}',
            bytes.fromhex("00112233445566778899aabbccddeeff"),
            struct.pack(">q", -105),
            None,
        ]
    query = ""
    parameter_types = b"\0\0"
    bound = b""
    while True:
        kind = receive(connection, 1)
        size = struct.unpack(">I", receive(connection, 4))[0]
        payload = receive(connection, size - 4)
        if kind == b"X":
            return
        if kind == b"Q":
            for statement in payload.rstrip(b"\0").split(b";"):
                if statement.strip():
                    send(b"C", statement.strip().split()[0].upper() + b"\0")
            send(b"Z", b"T")
            continue
        if kind == b"P":
            parts = payload.split(b"\0", 2)
            query = parts[1].decode()
            parameter_types = parts[2]
            send(b"1")
        elif kind == b"B":
            rest = payload.split(b"\0", 2)[2]
            formats = struct.unpack_from(">H", rest)[0]
            offset = 2 + formats * 2
            count = struct.unpack_from(">H", rest, offset)[0]
            offset += 2
            if count:
                length = struct.unpack_from(">i", rest, offset)[0]
                bound = rest[offset + 4 : offset + 4 + length]
            send(b"2")
        elif kind == b"D":
            if payload[:1] == b"S":
                send(b"t", parameter_types)
            if query.upper().startswith("SELECT"):
                columns = struct.pack(">H", len(fields))
                for name, oid, width in fields:
                    columns += name + b"\0" + struct.pack(
                        ">IHIhih", 0, 0, oid, width, -1, 1
                    )
                send(b"T", columns)
            else:
                send(b"n")
        elif kind == b"E":
            if query.upper().startswith("SELECT"):
                if stall:
                    while connection.recv(1024):
                        pass
                    return
                if not empty:
                    cells = (
                        values
                        if values is not None
                        else [bound, struct.pack(">q", 42)]
                    )
                    if all_null:
                        cells = [None] * len(fields)
                    send(
                        b"D",
                        struct.pack(">H", len(cells))
                        + b"".join(
                            struct.pack(">i", -1)
                            if cell is None
                            else struct.pack(">i", len(cell)) + cell
                            for cell in cells
                        ),
                    )
                send(b"C", b"SELECT 1\0")
            else:
                send(b"C", query.split()[0].upper().encode() + b"\0")
        elif kind == b"S":
            send(b"Z", b"T")
        elif kind == b"C":
            send(b"3")


def mysql(connection):
    def send(payload, sequence):
        connection.sendall(
            len(payload).to_bytes(3, "little") + bytes([sequence]) + payload
        )

    def packet():
        header = receive(connection, 4)
        return receive(connection, int.from_bytes(header[:3], "little"))

    def text(value):
        return bytes([len(value)]) + value

    def column(name, kind):
        return (
            text(b"def")
            + text(b"fixture")
            + text(b"")
            + text(b"")
            + text(name)
            + text(name)
            + b"\x0c"
            + struct.pack("<HIBHB", 33, 255, kind, 0, 0)
            + b"\0\0"
        )

    eof = b"\xfe\0\0\x02\0"
    ok = b"\0\0\0\x02\0\0\0"
    capabilities = 0x0008820D if legacy else 0x0108820D
    greeting = (
        b"\x0a8.0.0\0"
        + struct.pack("<I", 1)
        + b"12345678\0"
        + struct.pack("<H", capabilities & 65535)
        + b"\x21\x02\0"
        + struct.pack("<H", capabilities >> 16)
        + b"\x15"
        + b"\0" * 10
        + b"abcdefghijkl\0mysql_native_password\0"
    )
    send(greeting, 0)
    packet()
    send(ok, 2)
    query = ""
    while True:
        payload = packet()
        command = payload[0]
        if command == 1:
            return
        if command == 3:
            send(ok, 1)
        elif command == 0x16:
            query = payload[1:].decode()
            count = query.count("?")
            send(b"\0" + struct.pack("<IHHBH", 1, 2, count, 0, 0), 1)
            sequence = 2
            for _ in range(count):
                send(column(b"parameter", 253), sequence)
                sequence += 1
            if legacy and count:
                send(eof, sequence)
                sequence += 1
            send(column(b"label", 253), sequence)
            send(column(b"amount", 8), sequence + 1)
            if legacy:
                send(eof, sequence + 2)
        elif command == 0x17:
            if stall:
                continue
            count = query.count("?")
            offset = 10 + (count + 7) // 8 + 1 + count * 2
            if payload[10] & 1:
                bound = None
            else:
                length = payload[offset]
                bound = payload[offset + 1 : offset + 1 + length]
            send(b"\x02", 1)
            send(column(b"label", 253), 2)
            send(column(b"amount", 8), 3)
            if legacy:
                send(eof, 4)
            row = b"\0\x04" if bound is None else b"\0\0" + text(bound)
            send(row + struct.pack("<q", 42), 5 if legacy else 4)
            send(eof, 6 if legacy else 5)
        elif command == 0x19:
            pass
        else:
            raise ValueError(f"unexpected fixture command: {command}")


with socket.socket() as server:
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    server.settimeout(30)
    print(server.getsockname()[1], flush=True)
    connection, _ = server.accept()
    with connection:
        connection.settimeout(30)
        try:
            (postgres if sys.argv[1] == "postgres" else mysql)(connection)
        except (EOFError, ConnectionResetError):
            pass
