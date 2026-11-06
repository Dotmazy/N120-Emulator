#requires -Version 7.2
param(
    [ValidateSet('arm64-v8a','x86_64')][string]$Abi = 'arm64-v8a',
    [Parameter(Mandatory)][string]$QtVersionRoot,
    [Parameter(Mandatory)][string]$Jdk,
    [string]$Sdk = "$env:LOCALAPPDATA/Android/Sdk",
    [string]$NdkVersion = '27.2.12479018',
    [string]$BuildToolsVersion = '36.1.0',
    [string]$CMake = 'cmake',
    [string]$Ninja = 'ninja',
    [string]$OpenSslRoot
)
$ErrorActionPreference = 'Stop'
function Run([string]$Program, [string[]]$Arguments) {
    if ([IO.Path]::GetExtension($Program) -eq '.exe') {
        # Wait for this tool's PID, without waiting on inherited Gradle-daemon output pipes.
        $startInfo = [Diagnostics.ProcessStartInfo]::new($Program)
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        foreach ($argument in $Arguments) { $startInfo.ArgumentList.Add($argument) }
        $toolProcess = [Diagnostics.Process]::Start($startInfo)
        $standardOutput = $toolProcess.StandardOutput.ReadToEndAsync()
        $standardError = $toolProcess.StandardError.ReadToEndAsync()
        $toolProcess.WaitForExit()
        $code = $toolProcess.ExitCode
        # A detached daemon can retain a stream after the actual build process has exited.
        if ($standardOutput.Wait(1000)) { Write-Host $standardOutput.Result }
        if ($standardError.Wait(1000)) { Write-Host $standardError.Result }
        $toolProcess.Dispose()
    } else {
        & $Program @Arguments
        $code = $LASTEXITCODE
    }
    if ($code -ne 0) { throw "$Program failed ($code)" }
}
$root = Split-Path $PSScriptRoot -Parent
$qtKit = if ($Abi -eq 'arm64-v8a') { 'android_arm64_v8a' } else { 'android_x86_64' }
$buildName = if ($Abi -eq 'arm64-v8a') { 'android-arm64' } else { 'android-x86_64' }
$build = Join-Path $root "build/$buildName"
$deps = Join-Path $root '.local-audit/android-tools'
if (!$OpenSslRoot) { $OpenSslRoot = Join-Path $deps 'openssl-3.5.9-android' }
New-Item -ItemType Directory -Force $deps | Out-Null
if (!(Test-Path -LiteralPath "$OpenSslRoot/$Abi/lib/libssl_3.so")) { throw 'Prepare current OpenSSL with scripts/prepare-openssl-android.ps1 first.' }
$ninjaProgram = (Get-Command $Ninja).Source
$env:JAVA_HOME = $Jdk
$env:ANDROID_SDK_ROOT = $Sdk
$env:ANDROID_NDK_ROOT = Join-Path $Sdk "ndk/$NdkVersion"
$env:GRADLE_USER_HOME = Join-Path $deps 'gradle'
Run $CMake @('-S',$root,'-B',$build,'-G','Ninja',
    "-DCMAKE_TOOLCHAIN_FILE=$QtVersionRoot/$qtKit/lib/cmake/Qt6/qt.toolchain.cmake",
    "-DQT_HOST_PATH=$QtVersionRoot/mingw_64", "-DCMAKE_MAKE_PROGRAM=$ninjaProgram",
    "-DQT_ANDROID_SDK_BUILD_TOOLS_REVISION=$BuildToolsVersion",
    '-DCMAKE_BUILD_TYPE=Release','-DBUILD_TESTING=OFF',"-DN120_ANDROID_OPENSSL_ROOT=$OpenSslRoot")
Run $CMake @('--build',$build,'--target','apk','--parallel','6')
$keystore = Join-Path $deps 'n120-development.p12'
if (!(Test-Path -LiteralPath $keystore)) {
    Run "$Jdk/bin/keytool.exe" @('-genkeypair','-keystore',$keystore,'-storetype','PKCS12',
        '-storepass','android','-keypass','android','-alias','n120-development','-keyalg','RSA',
        '-keysize','3072','-validity','3650','-dname','CN=N120 Local Development')
}
$output = Join-Path $root "dist/android/n120-studio-$Abi.apk"
New-Item -ItemType Directory -Force (Split-Path $output) | Out-Null
$unsigned = Join-Path $build 'platforms/android/android-build/build/outputs/apk/release/android-build-release-unsigned.apk'
Run "$Sdk/build-tools/$BuildToolsVersion/zipalign.exe" @('-P','16','-f','4',$unsigned,$output)
Run "$Sdk/build-tools/$BuildToolsVersion/apksigner.bat" @('sign','--ks',$keystore,'--ks-key-alias','n120-development','--ks-pass','pass:android',$output)
Run "$Sdk/build-tools/$BuildToolsVersion/apksigner.bat" @('verify','--verbose',$output)
Get-FileHash -LiteralPath $output -Algorithm SHA256
Write-Host "Installable APK (local development signature): $output"
