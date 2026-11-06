param(
    [Parameter(Mandatory)][string]$Renode,
    [string]$HostExecutable,
    [string]$Storage,
    [string]$OpenSsl = 'openssl',
    [string]$Listen = '127.0.0.1',
    [int]$Port = 7420
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if (!$HostExecutable) { $HostExecutable = Join-Path $root 'build/cross-windows/n120-host.exe' }
if (!$Storage) { $Storage = Join-Path $root '.local-audit/remote-host' }
New-Item -ItemType Directory -Force $Storage | Out-Null
$key = Join-Path $Storage 'host-key.pem'
$certificate = Join-Path $Storage 'host-cert.pem'
$token = Join-Path $Storage 'pair-token.txt'
if (!(Test-Path -LiteralPath $key) -or !(Test-Path -LiteralPath $certificate)) {
    & $OpenSsl req -x509 -newkey rsa:3072 -sha256 -nodes -keyout $key -out $certificate -days 365 -subj '/CN=N120 Private Host'
    if ($LASTEXITCODE -ne 0) { throw 'Certificate generation failed' }
}
if (!(Test-Path -LiteralPath $token)) {
    $random = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    $rng.GetBytes($random); $rng.Dispose()
    [IO.File]::WriteAllText($token,([BitConverter]::ToString($random).Replace('-','').ToLowerInvariant()))
}
# Restrict credential files to this Windows user. Never print the token into logs.
if ($env:OS -eq 'Windows_NT') {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    foreach ($privateFile in @($key,$token)) {
        & icacls.exe $privateFile /inheritance:r /grant:r "${identity}:(F)" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Cannot secure $privateFile" }
    }
}
Write-Host "Pairing token: open $token privately and enter it on your phone."
& $HostExecutable --renode $Renode --storage (Join-Path $Storage 'session') --cert $certificate --key $key --token-file $token --listen $Listen --port $Port
if ($LASTEXITCODE -ne 0) { throw "Host exited ($LASTEXITCODE)" }
