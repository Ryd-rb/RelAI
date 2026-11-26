[CmdletBinding()]
param(
    [string]$Data = (Join-Path $env:LOCALAPPDATA 'RelAI'),
    [int]$Port = 0,
    [ValidateSet('relay','activity','graph','tools','models','dictionary','open')]
    [string]$View,
    [switch]$Headless,
    [switch]$ExternalProvider,
    [switch]$CheckProvider
)
$ErrorActionPreference = 'Stop'
if ($View -and ($Headless -or $CheckProvider -or $PSBoundParameters.ContainsKey('Port'))) {
    throw '-View selects a native window. Use it with the desktop launcher.'
}
$root = $PSScriptRoot
$manifest = Join-Path $root 'Cargo.toml'
$exe = Join-Path $root 'target/release/fluxr-app-host.exe'
cargo build --release --locked --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'FluxR host build failed.' }
$service = $null
$sessionDir = $null
try {
    if (!$ExternalProvider) {
        $codex = (Get-Command codex -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
        $sessionDir = Join-Path $Data ('provider-' + [Guid]::NewGuid().ToString('N'))
        $sessionDir = [IO.Directory]::CreateDirectory($sessionDir).FullName
        $tokenPath = Join-Path $sessionDir 'socket.token'
        $token = [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
        [IO.File]::WriteAllText($tokenPath, $token)
        $hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($token))).ToLowerInvariant()
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
        $listener.Start()
        $servicePort = $listener.LocalEndpoint.Port
        $listener.Stop()
        $env:RELAI_CODEX_URL = "ws://127.0.0.1:$servicePort"
        $env:RELAI_CODEX_TOKEN_FILE = $tokenPath
        $service = Start-Process -FilePath $codex -ArgumentList @('app-server','--listen',$env:RELAI_CODEX_URL,'--ws-auth','capability-token','--ws-token-sha256',$hash) -WorkingDirectory $sessionDir -PassThru -WindowStyle Hidden
        $ready=$false
        for ($i=0;$i -lt 60 -and !$service.HasExited;$i++){
            try { $ready=(Invoke-WebRequest "http://127.0.0.1:$servicePort/readyz" -TimeoutSec 1).StatusCode -eq 200 } catch {}
            if($ready){break}
            Start-Sleep -Milliseconds 250
        }
        if(!$ready -or $service.HasExited){throw 'Codex App Server did not become ready.'}
    }
    if($CheckProvider){cargo test --release --manifest-path $manifest --locked --test codex_live -- --ignored --nocapture}
    elseif ($Headless -or $PSBoundParameters.ContainsKey('Port')) { & $exe (Join-Path $root 'app') $Data --headless $Port }
    elseif ($View) { & $exe (Join-Path $root 'app') $Data --view $View.ToLowerInvariant() }
    else { & $exe (Join-Path $root 'app') $Data }
    if ($LASTEXITCODE -ne 0) { throw 'RelAI exited with an error.' }
} finally {
    if ($service -and !$service.HasExited) { $service.Kill(); $service.WaitForExit() }
    if ($sessionDir) {
        $tokenPath = Join-Path $sessionDir 'socket.token'
        if ([IO.File]::Exists($tokenPath)) { [IO.File]::Delete($tokenPath) }
        if ([IO.Directory]::Exists($sessionDir) -and !(Get-ChildItem -LiteralPath $sessionDir -Force)) { [IO.Directory]::Delete($sessionDir) }
    }
}
