#!/usr/bin/env python3
"""Collect local Mac network details and make optional read-only SynScan queries.

The protocol request list is deliberately fixed to documented Sky-Watcher
inquiries. This tool never sends motion, initialization, tracking, or guide
commands. It uses only the Python standard library and does not require Internet
access.
"""

from __future__ import annotations

import argparse
import datetime as dt
import ipaddress
import os
import platform
import re
import shutil
import socket
import subprocess
import sys
from pathlib import Path


DEFAULT_HOST = "192.168.4.1"
DEFAULT_PORT = 11880
MAX_COMMAND_TIMEOUT = 3.0
LOCAL_COMMAND_TIMEOUT = 2.0

# Official Sky-Watcher inquiries: ':e' motor-board version, ':a' grid per
# revolution, and ':s' PEC/worm data. The axis digit and CR terminator are part of
# the protocol. Axis 2 requests are opt-in because the common version query is
# axis 1 and only its answer is needed to identify the controller version.
AXIS_1_REQUESTS = (
    ("Motor board version (axis 1)", b":e1\r"),
    ("Grid per revolution (axis 1)", b":a1\r"),
    ("PEC/worm data (axis 1)", b":s1\r"),
)
AXIS_2_REQUESTS = (
    ("Grid per revolution (axis 2)", b":a2\r"),
    ("PEC/worm data (axis 2)", b":s2\r"),
)
COMMAND_SOURCE = (
    "https://github.com/skywatcher-pacific/skywatcher_open/blob/master/"
    "BasicApi_CS/Mount_Skywatcher.cs"
)
INDI_DECODING_SOURCE = (
    "https://github.com/indilib/indi/blob/master/drivers/telescope/skywatcherAPI.cpp"
)
INDI_DECODING_HEADER = (
    "https://github.com/indilib/indi/blob/master/drivers/telescope/skywatcherAPI.h"
)
REPLY_PATTERN = re.compile(rb"=([0-9A-Fa-f]{6})\r")


def parse_ipv4(value: str) -> str:
    try:
        address = ipaddress.ip_address(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("must be an IPv4 address") from exc
    if not isinstance(address, ipaddress.IPv4Address):
        raise argparse.ArgumentTypeError("must be an IPv4 address")
    return str(address)


def parse_port(value: str) -> int:
    try:
        port = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("must be an integer from 1 to 65535") from exc
    if not 1 <= port <= 65535:
        raise argparse.ArgumentTypeError("must be an integer from 1 to 65535")
    return port


def parse_timeout(value: str) -> float:
    try:
        timeout = float(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("must be a number from 0.1 to 3 seconds") from exc
    if not 0.1 <= timeout <= MAX_COMMAND_TIMEOUT:
        raise argparse.ArgumentTypeError("must be a number from 0.1 to 3 seconds")
    return timeout


def run_local_command(*arguments: str) -> tuple[str, str | None]:
    """Run a short local inspection command without a shell or raw diagnostics."""
    executable = shutil.which(arguments[0])
    if executable is None:
        return "", f"{arguments[0]} command not found"
    try:
        result = subprocess.run(
            [executable, *arguments[1:]],
            capture_output=True,
            text=True,
            timeout=LOCAL_COMMAND_TIMEOUT,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return "", f"{arguments[0]} timed out after {LOCAL_COMMAND_TIMEOUT:g} seconds"
    except OSError as exc:
        return "", f"{arguments[0]} could not run: {exc.__class__.__name__}"

    output = result.stdout.strip()
    if result.returncode != 0:
        return output, f"{arguments[0]} exited with status {result.returncode}"
    return output, None


def find_wifi_interface() -> tuple[str | None, str | None]:
    output, error = run_local_command("networksetup", "-listallhardwareports")
    if error:
        return None, error

    current_port = None
    for line in output.splitlines():
        if line.startswith("Hardware Port:"):
            current_port = line.partition(":")[2].strip().casefold()
        elif current_port in {"wi-fi", "airport"} and line.startswith("Device:"):
            return line.partition(":")[2].strip(), None
    return None, "networksetup did not report a Wi-Fi hardware interface"


def parse_route(output: str) -> dict[str, str]:
    fields: dict[str, str] = {}
    for line in output.splitlines():
        match = re.match(r"\s*(destination|gateway|interface|flags):\s*(.*?)\s*$", line)
        if match:
            fields[match.group(1)] = match.group(2)
    return fields


def decode_response(request: bytes, reply: bytes) -> tuple[str, int | None]:
    """Decode only strict normal replies for the fixed read-only inquiries."""
    match = REPLY_PATTERN.fullmatch(reply)
    if match is None:
        if reply.startswith(b"!"):
            return "not decoded: error response marker retained as raw data", None
        if reply.startswith(b"="):
            return "not decoded: expected '=' + six hex characters + CR", None
        return "not decoded: expected a documented normal '=' response", None

    function = request[1:2]
    if function not in {b"e", b"a", b"s"}:
        return "not decoded: inquiry type is not in the fixed decoder", None

    # Sky-Watcher BCDstr2long treats each two-character byte as hex and
    # assigns successive bytes increasing powers of 256 (little-endian).
    reply_bytes = bytes.fromhex(match.group(1).decode("ascii"))
    value = int.from_bytes(reply_bytes, byteorder="little")

    if function == b"e":
        # Match Sky-Watcher's motor-board version byte swap.
        version = (
            ((value & 0x0000FF) << 16)
            | (value & 0x00FF00)
            | ((value & 0xFF0000) >> 16)
        )
        major = (version >> 16) & 0xFF
        minor = (version >> 8) & 0xFF
        patch = version & 0xFF
        return (
            f"motor-board version after official byte reorder: 0x{version:06X} "
            f"(bytes {major:02X}.{minor:02X}.{patch:02X})",
            None,
        )

    if function == b"a":
        return f"controller microsteps per axis revolution: {value:,}", value

    return f"controller microsteps per worm revolution: {value:,}", value


def network_report(host: str) -> tuple[list[str], str | None]:
    lines = ["Local Mac and target network details:"]
    lines.append(f"  OS: {platform.system()} {platform.mac_ver()[0]} ({platform.machine()})")
    lines.append(f"  Python: {platform.python_version()}")

    wifi_interface, wifi_error = find_wifi_interface()
    if wifi_interface:
        lines.append(f"  Wi-Fi interface: {wifi_interface}")
        network, network_error = run_local_command(
            "networksetup", "-getairportnetwork", wifi_interface
        )
        if network and network.startswith("Current Wi-Fi Network:"):
            ssid = network.partition(":")[2].strip()
            lines.append(f"  Current Wi-Fi network (SSID): {ssid or '(empty)'}")
        elif network and "not associated" in network.casefold():
            lines.append(
                "  SSID via networksetup: not reported (command said not associated; "
                "verify in the macOS Wi-Fi status menu if it shows a connection)"
            )
        elif network:
            lines.append(f"  SSID via networksetup: unavailable ({network})")
        else:
            lines.append(f"  SSID via networksetup: unavailable ({network_error or 'no output'})")

        wifi_ip, ip_error = run_local_command("ipconfig", "getifaddr", wifi_interface)
        if wifi_ip:
            lines.append(f"  Wi-Fi IPv4 address: {wifi_ip.splitlines()[0]}")
        else:
            lines.append(f"  Wi-Fi IPv4 address: unavailable ({ip_error or 'no output'})")
    else:
        lines.append(f"  Wi-Fi interface: unavailable ({wifi_error or 'not identified'})")

    lines.append(f"  Probe target: {host}")
    route_output, route_error = run_local_command("route", "-n", "get", host)
    route = parse_route(route_output)
    if route:
        lines.append("  Target route:")
        for field in ("destination", "gateway", "interface", "flags"):
            if field in route:
                lines.append(f"    {field}: {route[field]}")
        route_interface = route.get("interface")
        if route_interface and route_interface != wifi_interface:
            route_ip, route_ip_error = run_local_command("ipconfig", "getifaddr", route_interface)
            if route_ip:
                lines.append(f"    route interface IPv4 address: {route_ip.splitlines()[0]}")
            else:
                lines.append(
                    "    route interface IPv4 address: unavailable "
                    f"({route_ip_error or 'no output'})"
                )
    else:
        lines.append(f"  Target route: unavailable ({route_error or 'no parsed route details'})")

    lines.append("  Privacy: no Wi-Fi password is queried; MAC addresses and BSSIDs are omitted from the report.")
    return lines, route.get("interface")


def make_request(host: str, port: int, timeout: float, label: str, request: bytes) -> dict[str, str]:
    result = {
        "label": label,
        "request": repr(request.decode("ascii")),
        "request_code": request[:3].decode("ascii"),
        "status": "",
        "local_ip": "unavailable",
        "reply_ascii": "",
        "reply_hex": "",
        "interpretation": "not decoded: no UDP reply",
        "decoded_microsteps": "",
    }
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.settimeout(timeout)
            # UDP connect selects a local route/address; it does not send a packet.
            sock.connect((host, port))
            result["local_ip"] = sock.getsockname()[0]
            sock.send(request)
            reply = sock.recv(65535)
    except socket.timeout:
        result["status"] = f"no UDP reply within {timeout:g} seconds"
    except OSError as exc:
        result["status"] = f"socket error: {exc.__class__.__name__}: {exc}"
    else:
        result["status"] = "UDP reply received"
        result["reply_ascii"] = repr(reply.decode("ascii", errors="backslashreplace"))
        result["reply_hex"] = reply.hex(" ")
        if reply.startswith(b"="):
            result["reply_marker"] = "'=' (documented normal-response marker)"
        elif reply.startswith(b"!"):
            result["reply_marker"] = "'!' (documented error-response marker)"
        else:
            result["reply_marker"] = "not a documented '=' or '!' marker"
        interpretation, decoded_microsteps = decode_response(request, reply)
        result["interpretation"] = interpretation
        if decoded_microsteps is not None:
            result["decoded_microsteps"] = str(decoded_microsteps)
    return result


def build_report(args: argparse.Namespace, timestamp: dt.datetime) -> str:
    lines = [
        "SynScan Wi-Fi local probe",
        f"Generated: {timestamp.astimezone().isoformat(timespec='seconds')}",
        f"Mode: {'network-only (no adapter protocol packets)' if args.network_only else 'read-only Sky-Watcher inquiries'}",
        "",
    ]

    network_lines, route_interface = network_report(args.host)
    lines.extend(network_lines)
    lines.append("")

    if args.network_only:
        lines.extend(
            [
                "Adapter protocol:",
                "  No UDP inquiry packets were sent (--network-only).",
                "  No conclusion about adapter or mount compatibility was attempted.",
            ]
        )
        return "\n".join(lines) + "\n"

    requests = list(AXIS_1_REQUESTS)
    if args.axis_2:
        requests.extend(AXIS_2_REQUESTS)
    results = [
        make_request(args.host, args.port, args.timeout, label, request)
        for label, request in requests
    ]
    lines.extend(
        [
            "Read-only adapter inquiries:",
            f"  Destination: {args.host}:{args.port}/UDP",
            f"  Per-request timeout: {args.timeout:g} seconds; retries: 0",
            f"  Route interface selected by macOS: {route_interface or 'not identified'}",
            "  Source: " + COMMAND_SOURCE,
            "  Decoder semantics: " + INDI_DECODING_SOURCE,
            "  Decoder field definitions: " + INDI_DECODING_HEADER,
            "  Decoder accepts only '=' + six hex digits + CR and parses hex byte pairs little-endian.",
            "  :e uses Sky-Watcher's version byte reorder; :a and :s use the INDI microstep labels.",
            "  Only the fixed read-only inquiry bytes below were sent.",
            "  No slew, guide, initialization, tracking, or setting commands were sent.",
        ]
    )
    for result in results:
        lines.extend(
            [
                f"\n  {result['label']}",
                f"    Request ASCII: {result['request']}",
                f"    Local source IPv4: {result['local_ip']}",
                f"    Result: {result['status']}",
            ]
        )
        if result.get("reply_ascii"):
            lines.append(f"    Reply ASCII (escaped): {result['reply_ascii']}")
            lines.append(f"    Reply hex: {result['reply_hex']}")
            lines.append(f"    Reply marker: {result['reply_marker']}")
        lines.append(f"    Interpretation: {result['interpretation']}")

    replies = sum(result["status"] == "UDP reply received" for result in results)
    timeouts = sum(result["status"].startswith("no UDP reply") for result in results)
    if replies == 0:
        if timeouts == len(results):
            result_line = "  NO UDP REPLY was received for any inquiry. This is inconclusive:"
        else:
            result_line = (
                "  No UDP reply was recorded; one or more inquiries also had a socket error,"
            )
        lines.extend(
            [
                "\nCompatibility interpretation:",
                result_line,
                "  so some requests may not have been sent. Silence or a local socket error",
                "  does not establish that the mount is incompatible. Adapter power, Wi-Fi",
                "  association, station/hotspot mode, target IP, route, UDP port, and protocol",
                "  support can all affect the result. Compatibility and pulse-guide support",
                "  remain undetermined.",
            ]
        )
    else:
        axis_grid = next(
            (result.get("decoded_microsteps") for result in results
             if result["request_code"] == ":a1"),
            "",
        )
        axis_worm = next(
            (result.get("decoded_microsteps") for result in results
             if result["request_code"] == ":s1"),
            "",
        )
        if axis_grid and axis_worm and int(axis_grid) > 0 and int(axis_worm) > 0:
            ratio = int(axis_grid) / int(axis_worm)
            ratio_line = (
                f"  Approximate axis/worm ratio (a1 divided by s1): {ratio:.6f} "
                "worm revolutions per axis revolution. This ratio may be evidence "
                "for an approximate worm-wheel tooth count; it is not a direct "
                "physical tooth count or a measured worm period."
            )
        else:
            ratio_line = (
                "  Approximate axis/worm ratio unavailable: both :a1 and :s1 must "
                "have valid, positive decoded replies."
            )
        lines.extend(
            [
                "\nCompatibility interpretation:",
                f"  Received {replies} UDP reply/replies. Raw bytes are recorded above;",
                "  strict, supported replies are decoded using the rules described above.",
                ratio_line,
                "  A reply does not confirm mount compatibility or pulse-guide support.",
                "  No automatic incompatible-mount verdict is made.",
            ]
        )
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Collect local Mac Wi-Fi/route details and optionally send fixed, "
            "read-only Sky-Watcher inquiries to a SynScan adapter."
        ),
        epilog=(
            "Default sends :e1, :a1, and :s1 inquiries only. "
            "Use --network-only to send no adapter protocol packets."
        ),
    )
    parser.add_argument("--host", type=parse_ipv4, default=DEFAULT_HOST, help="adapter IPv4 address (default: 192.168.4.1)")
    parser.add_argument("--port", type=parse_port, default=DEFAULT_PORT, help="adapter UDP port (default: 11880)")
    parser.add_argument(
        "--timeout",
        type=parse_timeout,
        default=1.0,
        help="per-inquiry timeout from 0.1 to 3 seconds (default: 1.0; one attempt each)",
    )
    parser.add_argument("--axis-2", action="store_true", help="also inquire axis 2 grid and PEC period")
    parser.add_argument("--network-only", action="store_true", help="collect local details without sending UDP protocol packets")
    parser.add_argument("--output", type=Path, help="report path (default: ~/Downloads/synscan_wifi_probe_<timestamp>.txt)")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    timestamp = dt.datetime.now().astimezone()
    if args.output:
        report_path = args.output.expanduser()
    else:
        report_path = Path.home() / "Downloads" / (
            "synscan_wifi_probe_" + timestamp.strftime("%Y%m%d_%H%M%S_%f") + ".txt"
        )

    try:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        # The report can contain a private SSID, local addresses, and raw
        # controller replies. Restrict access even under a permissive umask.
        report_fd = os.open(report_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(report_fd, "w", encoding="utf-8") as report_file:
            report_file.write(build_report(args, timestamp))
    except OSError as exc:
        print(f"Could not write report: {exc}", file=sys.stderr)
        return 2

    print(f"Report saved to: {report_path}")
    if args.network_only:
        print("Network-only mode sent no adapter protocol packets.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
