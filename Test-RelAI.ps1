[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust formatting check failed.' }
    cargo clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
    cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Offline test suite failed.' }
} finally {
    Pop-Location
}
