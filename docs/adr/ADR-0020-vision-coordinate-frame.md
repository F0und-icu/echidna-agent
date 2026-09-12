# ADR-0020: Bind Vision Regions to the Delivered Image

- Status: ACCEPTED for operator-authorized F6a implementation
- Architecture version: 1.10.0
- Supersedes: implicit coordinate-frame assumptions for new secondary Vision observations

F5 Live correctly recognized colors in a 32×16 image but returned 240×240
regions. Existing positive-width validation cannot detect this. Prompt wording
alone cannot establish a coordinate contract.

New secondary requests carry typed original/delivery dimensions and the selected
delivery Artifact, resolved from immutable image bytes in the image Adapter.
The existing ContextPlanner records the same metadata in RequestAudit. Regions
use integer pixels in the delivered image, top-left origin, with exclusive
right/bottom edges. They never implicitly refer to the original image or device
screen. No automatic conversion, clamping, hidden retry or Provider switch.

Observation payload v2 requires this geometry. One pure Domain owner validates
frame consistency and every region's bounds using overflow-safe arithmetic.
Empty regions are valid when localization is unavailable. A completed reply
with invalid regions creates the existing failed-attempt terminal, not a usable
observation; its raw Blob is retained and identified in the diagnostic.

The Session envelope, routes, failures and model reply retain their existing
versions. Observation v1 is read without guessed geometry; it remains v1 and
its context says `unknown_legacy`. This is explicit dual-version reading, not
an in-place migration or a synthetic upcast that invents historical metadata.
New runtime production writes v2 only. Mechanical Replay compares v2 geometry
against the reconstructed RequestAudit as well as raw model content; legacy
replay stays read-only and unchanged in authority.

Older binaries that only support Observation v1 cannot resume sessions containing
v2; they must fail on the unsupported payload instead of silently dropping it.
Do not downgrade such sessions or erase geometry to make them readable. The
model's JSON reply remains schema v1 and is independently versioned from the
runtime observation; no model is asked to invent authoritative image metadata.

Bounds validity proves only that a rectangle fits the declared image. It does
not prove the model identified the right object. Every Vision observation,
including in-bounds v2, remains unverified and non-evidence. Primary Direct is
unchanged and does not acquire a structured localization guarantee.

Reuse existing image inspection, Artifact/Blob, ContextPlanner and Vision
owners. No new dependency, Port, fact store, crop engine, coordinate router or
model-specific behavior. Test through production consumers; no test-only API.
