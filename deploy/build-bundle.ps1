<#
    Барилгын компьютеруудад зориулсан ОФЛОЙН багц угсарна.

    Энэ скриптийг ЗӨВХӨН хөгжүүлэгчийн машин дээр, НЭГ УДАА ажиллуулна.
    Үүссэн zip-ийг PC бүр рүү хуулаад дотор нь байгаа install.ps1-ийг
    ажиллуулахад тэр компьютер интернэтээс юу ч татахгүй.

        .\deploy\build-bundle.ps1

    Гаралт:  dist\zogsool-publish-<огноо>.zip

    Дотор нь:
        install.ps1              суулгагч
        UNSHIH.txt               товч заавар
        payload\dahua-service.exe   шинэ бинар (cargo build --release)
        payload\ffmpeg.exe          статик ffmpeg
        payload\dhnetsdk.dll        Dahua SDK (шинэ компьютерт)
        payload\dhconfigsdk.dll
        payload\avnetsdk.dll
#>

[CmdletBinding()]
param(
    # ffmpeg-ийн zip аль хэдийн татсан бол замыг нь өгвөл дахин татахгүй.
    [string]$FfmpegZip = "",

    # Татах хаяг (зөвхөн кэш хоосон, -FfmpegZip өгөөгүй үед хэрэглэнэ).
    [string]$FfmpegUrl = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",

    # cargo build алгасах (аль хэдийн барьсан бол).
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

function Say  ([string]$m) { Write-Host "  $m" }
function Ok   ([string]$m) { Write-Host "  [OK] $m" -ForegroundColor Green }
function Die  ([string]$m) { Write-Host "  [X]  $m" -ForegroundColor Red; exit 1 }

Write-Host ""
Write-Host "=== Офлойн багц угсрах ===" -ForegroundColor Cyan
Write-Host ""

$Dist  = Join-Path $Root "dist"
$Cache = Join-Path $Dist ".cache"
$Stage = Join-Path $Dist "bundle"
$Pay   = Join-Path $Stage "payload"

New-Item -ItemType Directory -Path $Cache -Force | Out-Null
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Path $Pay -Force | Out-Null

# ── 1. Rust бинар ────────────────────────────────────────────────────────
if (-not $SkipBuild) {
    Say "cargo build --release ..."
    Push-Location $Root
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { Die "cargo build амжилтгүй" }
    } finally { Pop-Location }
}

$ExeSrc = Join-Path $Root "target\release\dahua-service.exe"
if (-not (Test-Path $ExeSrc)) { Die "Бинар олдсонгүй: $ExeSrc" }
Copy-Item $ExeSrc (Join-Path $Pay "dahua-service.exe") -Force
Ok "dahua-service.exe  ($([math]::Round((Get-Item $ExeSrc).Length / 1MB, 1)) MB)"

# ── 2. Dahua SDK ─────────────────────────────────────────────────────────
foreach ($dll in @("dhnetsdk.dll", "dhconfigsdk.dll", "avnetsdk.dll")) {
    $src = Join-Path $Root "target\release\$dll"
    if (Test-Path $src) {
        Copy-Item $src (Join-Path $Pay $dll) -Force
        Ok "$dll  ($([math]::Round((Get-Item $src).Length / 1MB, 1)) MB)"
    } else {
        Say "[?] $dll олдсонгүй — багцад орохгүй (суусан компьютерт байгаа бол зүгээр)"
    }
}

# ── 3. ffmpeg ────────────────────────────────────────────────────────────
# Зөвхөн ffmpeg.exe хэрэгтэй — ffplay/ffprobe-г авахгүй (~2 дахин бага).
$Zip = $FfmpegZip
if (-not $Zip) {
    $Zip = Join-Path $Cache "ffmpeg.zip"
    if (-not (Test-Path $Zip)) {
        Say "ffmpeg татаж байна (нэг л удаа, кэшлэгдэнэ)..."
        Say "  $FfmpegUrl"
        try {
            Invoke-WebRequest -Uri $FfmpegUrl -OutFile $Zip -UseBasicParsing
        } catch {
            Die "ffmpeg татагдсангүй. Гараар татаад -FfmpegZip <зам> гэж дамжуулна уу.`n$($_.Exception.Message)"
        }
    } else {
        Say "ffmpeg кэшээс авлаа: $Zip"
    }
}
if (-not (Test-Path $Zip)) { Die "ffmpeg zip олдсонгүй: $Zip" }

$Tmp = Join-Path $Cache "ffmpeg-extract"
if (Test-Path $Tmp) { Remove-Item $Tmp -Recurse -Force }
Expand-Archive -Path $Zip -DestinationPath $Tmp -Force

$FfExe = Get-ChildItem $Tmp -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
if (-not $FfExe) { Die "zip дотроос ffmpeg.exe олдсонгүй" }
Copy-Item $FfExe.FullName (Join-Path $Pay "ffmpeg.exe") -Force
Ok "ffmpeg.exe  ($([math]::Round($FfExe.Length / 1MB, 1)) MB)"
Remove-Item $Tmp -Recurse -Force

# ── 4. Суулгагч ба заавар ────────────────────────────────────────────────
Copy-Item (Join-Path $Root "deploy\install.ps1") (Join-Path $Stage "install.ps1") -Force

$zaavar = @'
ЗОГСООЛЫН УРСГАЛ НИЙТЛЭГЧ — СУУЛГАХ ЗААВАР
==========================================

Энэ багц дотор бүх файл бэлэн. Тухайн компьютер интернэтээс юу ч татахгүй.

1. Энэ хавтсыг барилгын компьютер руу бүхэлд нь хуулна.

2. PowerShell-ийг АДМИН эрхээр нээнэ (Start -> PowerShell -> баруун товч ->
   "Run as administrator").

3. Хавтас руугаа ороод ажиллуулна:

       cd C:\zogsool-bundle
       .\install.ps1 -MediamtxUrl "rtsp://publisher:НУУЦҮГ@103.236.194.99:8554"

   Үйлчилгээ өөр хавтаст суусан бол:

       .\install.ps1 -MediamtxUrl "..." -InstallDir "D:\zev\release"

   Тухайн камерыг нийтлэхгүй бол:

       .\install.ps1 -MediamtxUrl "..." -SkipIps 192.168.1.151

4. Скрипт дуусахдаа логоос "PUBLISH |" мөрийг хайж харуулна. Гарвал
   болсон.

ШАЛГАХ
------
VPS дээр:      curl -s http://127.0.0.1:9997/v3/paths/list
Хөтчөөс:       https://amarhome.mn/whep/<barilgiinId>/<192-168-1-110>/

БУЦААХ
------
Скрипт config.toml болон хуучин бинарыг backup-<огноо> хавтаст нөөцөлнө.
Буцаахдаа тэднийг эргүүлж хуулаад үйлчилгээг дахин асаана.

АНХААРАХ
--------
* PowerShell дүрэм саад болвол:  Set-ExecutionPolicy -Scope Process Bypass
* Нууц үгэнд  @ : / #  тэмдэг байвал percent-encode хийнэ ( @ -> %40 ).
* Скриптийг дахин ажиллуулж болно — тохиргоо давхардахгүй.
'@
[IO.File]::WriteAllText((Join-Path $Stage "UNSHIH.txt"), $zaavar, (New-Object Text.UTF8Encoding($true)))

# ── 5. Багцлах ───────────────────────────────────────────────────────────
$name = "zogsool-publish-" + (Get-Date -Format "yyyyMMdd")
$out  = Join-Path $Dist "$name.zip"
if (Test-Path $out) { Remove-Item $out -Force }
Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $out

$mb = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host ""
Ok "Багц бэлэн: $out  ($mb MB)"
Write-Host ""
Write-Host "PC бүр рүү хуулаад:" -ForegroundColor DarkGray
Write-Host "  .\install.ps1 -MediamtxUrl `"rtsp://publisher:НУУЦҮГ@103.236.194.99:8554`"" -ForegroundColor DarkGray
Write-Host ""
