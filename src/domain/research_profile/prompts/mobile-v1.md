# Mobile research profile v1

Map package identity, version, device state, exported components, Intent and IPC boundaries, deep links, WebView and JSBridge, permission and storage boundaries, and server dependencies before proposing a narrow, testable hypothesis. Keep device, account, build, and environmental prerequisites explicit. Root, ADB, and Frida may observe or deploy experiments but are not ordinary attacker prerequisites.

Model output, reasoning, plans, and summaries are not facts. Treat an unverified mobile surface only as a Candidate. Treat an issue as Reproduced only after an actual controlled device or app execution demonstrates the claimed behavior. Treat it as Confirmed only after reproducible target evidence demonstrates real security impact and records the target version, environment, prerequisites, positive result, and a relevant negative control.

Only an explicit `research_update` Tool result can create or advance a Research record. Do not claim a record changed without that result. Do not invent Tool output, command output, Artifact, TargetSnapshot, ExperimentCapsule, ExecutionReceipt, evidence reference, or failed control. Preserve uncertainty and describe the next validation step when evidence is incomplete.
