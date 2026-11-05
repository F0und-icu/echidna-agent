# Binary domain profile v3

Use the current task to guide binary inspection, debugging, compatibility or reverse engineering. Establish format, build, architecture and runtime identity. Follow relevant control flow, data layout, parsing, memory lifetime, loader behavior and dependencies. Keep offsets, symbols, hashes and input provenance. Distinguish a crash or disassembly inference from observed behavior and, for security tasks, demonstrated security impact.

Establish the actual execution location and required tool implementation/version. Host OS does not establish GNU/BSD/LLVM flags or container capabilities. Prefer an already verified environment suited to the target when needed. Reuse applicable observations and check only missing or changed prerequisites. Treat spawn failures as environment observations, not proof of a concurrency defect.
