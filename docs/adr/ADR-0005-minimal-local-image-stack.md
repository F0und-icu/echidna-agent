# ADR-0005: Minimal Local Image Stack

- Status: ACCEPTED
- Date: 2026-08-26
- Deciders: project owner
- Architecture version: 1.1.0 implementation decision
- Related Slice: M3-S4
- Supersedes: none
- Superseded by: none

## Context

M3-S4 must accept a local PNG or JPEG as an auditable Artifact, reject unsafe
or malformed input before it enters a model request, and create a deterministic
budgeted derivative when the source exceeds the selected image budget. The
existing Artifact Store owns durable bytes and SHA-256 identity; it does not
decode images. The domain must remain filesystem and decoder independent.

## Decision

- Add `image 0.25` with default features disabled and only `png` and `jpeg`
  codecs enabled. It is used only by the local image Adapter for decoding,
  resizing and deterministic PNG encoding.
- Add `base64 0.22` only in the OpenAI Responses Adapter to encode an already
  resolved immutable image Blob into a `data:<media-type>;base64,...` URL.
- Accept exactly PNG and JPEG. Reject a symlink, non-regular path, unsupported
  signature, malformed decoder input, byte limit, pixel limit, dimension limit
  or output-budget failure explicitly before any Artifact or Provider fact is
  published.
- Retain original bytes unchanged. A derived PNG is created only when the
  requested byte/pixel budget requires it and is linked to its original by the
  existing Artifact Lineage fields with the fixed transform identifier
  `image_budget_v1`.
- The pure Domain owns limits, format names, metadata and the deterministic
  resize target calculation. The Adapter owns paths, decode/encode and bytes.

## Alternatives

- Platform commands (`sips`, ImageMagick): rejected because availability,
  output and flags differ by host and cannot provide one portable audited
  contract.
- Copying Codex image code: rejected. The inspected upstream files remain a
  REFERENCE; this small adapter is independently reimplemented and creates no
  upstream Port Ledger entry.
- A broad multimedia library or image cache: rejected as outside the local
  image-input-only scope.

## Consequences

The locked dependency closure adds codec compilation cost but avoids system
tool coupling. It is removable when image input is removed or a smaller,
maintained decoder can demonstrate the same PNG/JPEG validation, dimensions,
deterministic derivative and data-URL behavior under the pinned toolchain.

No frozen architecture document, Provider-neutral ModelInputIR schema or
Artifact Blob identity changes. No image generation, video, audio or remote
image processing is introduced.
