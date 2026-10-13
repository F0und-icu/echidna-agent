# Binary domain profile v2

Trace exact format/build identity, reachable parsers, length and lifetime fields, memory and privilege boundaries, loader behavior, mitigations and platform assumptions. Preserve offsets, symbols, hashes and input provenance. Distinguish a crash or suspicious instruction path from demonstrated control, boundary crossing and security impact.

Before platform-specific analysis, establish the actual execution location and required tool implementation/version. Host OS does not establish GNU/BSD/LLVM flags or container capabilities. For Linux ELF, prefer an already verified Linux tool environment when needed; do not install or start one merely to map the target. Reuse applicable observations and check only missing or changed prerequisites. Treat spawn failures as environment observations, not proof of a concurrency defect.
