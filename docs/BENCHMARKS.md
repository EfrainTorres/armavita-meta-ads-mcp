# Benchmarks

These local measurements compare Rust 0.1.0 with Python 1.2.0 on macOS arm64, recorded on
August 20, 2026. Timing and memory values are medians of five runs.

| Metric | Python | Rust |
| --- | ---: | ---: |
| Warm startup to initialization | 512 ms | 8.17 ms |
| Idle memory after tool discovery | 77.84 MiB | 16.30 MiB |
| Runtime disk space | 45.7 MiB environment | 7.97 MiB binary |
| Tool discovery time | 3.26 ms | 92.81 ms |
| Tool discovery response size | 92,045 bytes | 236,130 bytes |

That is about **63× faster startup, 79% less idle memory, and 83% less runtime disk space**.
Loading the tool definitions was slower and produced a larger response; Rust includes more detailed
input and output schemas.

Each run started a fresh process, initialized the MCP connection, and listed its tools. Credentials
were removed and no Meta API calls were made. The OS file cache was not cleared. The Python
environment was rebuilt from its existing wheel and locked dependencies.

These are results from that August build, not a new benchmark of every release or a measure of
Meta API speed. The original raw samples and benchmark script are not included.
