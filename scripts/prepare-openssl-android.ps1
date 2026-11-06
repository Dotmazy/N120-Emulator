param(
    [Parameter(Mandatory)][string]$MsysRoot,
    [string]$Sdk = "$env:LOCALAPPDATA/Android/Sdk",
    [string]$NdkVersion = '27.2.12479018'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$deps = Join-Path $root '.local-audit/android-tools'
$version = '3.5.9'
$archive = Join-Path $deps "openssl-$version.tar.gz"
$expected = '603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a'
New-Item -ItemType Directory -Force $deps | Out-Null
if (!(Test-Path -LiteralPath $archive)) {
    Invoke-WebRequest "https://github.com/openssl/openssl/releases/download/openssl-$version/openssl-$version.tar.gz" -OutFile $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'OpenSSL source checksum mismatch' }
$source = Join-Path $deps "openssl-$version"
if (!(Test-Path -LiteralPath "$source/Configure")) {
    & tar -xzf $archive -C $deps
    if ($LASTEXITCODE -ne 0) { throw 'OpenSSL extraction failed' }
}
# Qt loads libssl_3.so/libcrypto_3.so. Keep the ordinary OPENSSL_* symbol versions.
$config = Join-Path $source 'Configurations/15-android.conf'
$text = [IO.File]::ReadAllText($config)
if (!$text.Contains('shlib_variant => "_3"')) {
    [IO.File]::WriteAllText($config,$text.Replace('shared_extension => ".so",',"shared_extension => `".so`",`n        shlib_variant => `"_3`","))
}
$definition = Join-Path $source 'util/mkdef.pl'
[IO.File]::WriteAllText($definition,[IO.File]::ReadAllText($definition).Replace('OPENSSL${SO_VARIANT}_','OPENSSL_'))
$bash = Join-Path $MsysRoot 'usr/bin/bash.exe'
# OpenSSL's generated makefiles require source/build paths without spaces. Alias only this checkout.
$drive = @('Z','Y','X','W','V','U','T') | Where-Object { !(Test-Path "${_}:/") } | Select-Object -First 1
if (!$drive) { throw 'No free temporary drive letter for the OpenSSL build' }
& subst "${drive}:" $root
if ($LASTEXITCODE -ne 0) { throw 'Cannot create workspace drive alias' }
try {
    $prefix = '/'+$drive.ToLowerInvariant()
    $ndk = Join-Path $Sdk "ndk/$NdkVersion"
    $ndkPosix = & $bash -c '/usr/bin/cygpath -u "$1"' -- $ndk
    if ($LASTEXITCODE -ne 0) { throw 'Cannot convert NDK path' }
    foreach ($pair in @(@('arm64','arm64-v8a'),@('x86_64','x86_64'))) {
        & $bash "$prefix/scripts/build-openssl-android.sh" "$prefix/.local-audit/android-tools/openssl-$version" `
            "$prefix/.local-audit/android-tools/openssl-build-$($pair[0])" `
            "$prefix/.local-audit/android-tools/openssl-$version-android/$($pair[1])" $ndkPosix $pair[0]
        if ($LASTEXITCODE -ne 0) { throw "OpenSSL $($pair[1]) build failed" }
    }
    Copy-Item -LiteralPath "$source/LICENSE.txt" -Destination "$deps/openssl-$version-android/LICENSE.txt"
} finally {
    & subst "${drive}:" /D
}
Write-Host "OpenSSL Android root: $deps/openssl-$version-android"
