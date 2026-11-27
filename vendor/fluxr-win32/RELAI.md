# RelAI native theme

This is the `crates/fluxr-win32` crate from
[FluxR-lang](https://github.com/Ryd-rb/FluxR-lang/tree/6fe5f1ce7cf77f672208b7e45e3e970eae3db0a7/crates/fluxr-win32),
revision `6fe5f1ce7cf77f672208b7e45e3e970eae3db0a7`, under its MIT license
(included as `LICENSE`). The manifest and source files are retained from that
revision. The upstream crate's lockfile is omitted; RelAI's root lockfile governs
the dependency resolution.

RelAI uses this local Windows-only dependency because the pinned upstream frame
ABI selects `system` or `dark` but has no palette customization hook. The core
FluxR language dependency remains pinned upstream. This crate is excluded from
the root workspace so upstream native window tests are not implicitly run by
RelAI's workspace checks.

The only source customization is the centralized palette block in
`src/windows/dark.rs`: graphite surfaces, a blue-violet primary action, pale
periwinkle focus and selection accents, and cool neutral text. The same tokens
cover branding, buttons, selected navigation, tabs, checkboxes and focus rings.
No layout, native window behavior, event handling or high-contrast fallback is
changed. System high contrast still uses standard Windows controls and colors.

When updating FluxR, replace this crate with the same new upstream revision as
the core dependency, retain its license and reapply only this palette block. If
upstream adds a supported palette API, remove this vendored crate and supply
these colors through that API instead.
