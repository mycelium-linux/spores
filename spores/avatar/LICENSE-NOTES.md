# License and dependency notes

Verified 2026-11-27 from npm registry metadata and upstream license files; no
package was installed or vendored.

| Library | Actual package/language | License | Result |
| --- | --- | --- | --- |
| DiceBear | `@dicebear/core` 11.0.0; TypeScript/JavaScript (npm `lib/index.js`, `lib/index.d.ts`) | MIT | Reusable in JS/TS; not used here. Style packs are separate packages with their own licenses, so they would need per-style review. |
| Boring Avatars | `boring-avatars` 2.0.4; JavaScript/TypeScript (npm `dist/index.js`, `dist/index.d.ts`) | MIT | Reusable in JS/TS; not used here. |
| Jdenticon | `jdenticon` 3.3.0; JavaScript (npm `dist/jdenticon-node.js`) | MIT | Reusable in JS/Node; not used here. |

Sources checked:

- `https://registry.npmjs.org/@dicebear/core/latest`
- `https://registry.npmjs.org/boring-avatars/latest`
- `https://registry.npmjs.org/jdenticon/latest`
- `https://raw.githubusercontent.com/boringdesigners/boring-avatars/master/LICENSE` (MIT)
- `https://raw.githubusercontent.com/dmester/jdenticon/master/LICENSE` (MIT)
- `https://raw.githubusercontent.com/dicebear/dicebear/main/packages/%40dicebear/core/package.json` (MIT)

Decision: no third-party runtime or visual asset is bundled. The Soil geometry
here is original. Existing Rakazo geometry (`FuturePresentLabs/fpl-agents`
`packages/core/src/avatar-shape.ts`, Apache-2.0) was inspected as prior art and
is **not** copied; the fold/orbit/signal geometry and palettes are new. The PNG
derivatives use the already-installed `rsvg-convert` (librsvg); it is a local
build tool, not a shipped service dependency. The contact-sheet compositor uses
Pillow only to assemble preview evidence.

No inference/image-generation model is used. No public-domain claim is made
about the marks; they are original synthetic geometry for this prototype.
