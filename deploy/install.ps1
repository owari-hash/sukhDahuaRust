<#
    Зогсоолын үйлчилгээг шинэчилж, камерын урсгалыг VPS рүү нийтлэх
    тохиргоог хийнэ. ОФЛАЙН ажиллана — багц дотор бүх файл бэлэн байгаа
    тул тухайн компьютер интернэтээс юу ч татахгүй.

    Ажиллуулах (админ эрхтэй PowerShell):

        .\install.ps1 -MediamtxUrl "rtsp://publisher:NUUTSUG@103.236.194.99:8554"

    Юу хийдэг вэ:
      1. Үйлчилгээг зогсооно
      2. config.toml-ийг нөөцөлнө
      3. ffmpeg.exe, шинэ dahua-service.exe, Dahua SDK dll-үүдийг хуулна
      4. config.toml-д [publish] хэсгийг нэмнэ/шинэчилнэ
      5. Үйлчилгээг асааж, логоос PUBLISH мөрийг хүлээнэ

    Дахин ажиллуулж болно — өмнөх тохиргоог давхардуулахгүй.
#>

[CmdletBinding()]
param(
    # rtsp://publisher:НУУЦҮГ@СЕРВЕР:8554  — MediaMTX-ийн нийтлэх хаяг.
    [Parameter(Mandatory = $true)]
    [string]$MediamtxUrl,

    # Үйлчилгээ суусан хавтас (config.toml, dahua-service.exe энд байна).
    [string]$InstallDir = "C:\zev\release",

    # Windows үйлчилгээний нэр. Хоосон бол автоматаар хайна.
    [string]$ServiceName = "",

    # Тухайн камерыг нийтлэхгүй байх бол IP-г энд жагсаана.
    [string[]]$SkipIps = @(),

    # Зөвхөн тохиргоо — файл хуулахгүй.
    [switch]$ConfigOnly,

    # Файл хуулаад тохиргоонд хүрэхгүй.
    [switch]$FilesOnly
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

function Say  ([string]$m) { Write-Host "  $m" }
function Ok   ([string]$m) { Write-Host "  [OK] $m"   -ForegroundColor Green }
function Warn ([string]$m) { Write-Host "  [!]  $m"   -ForegroundColor Yellow }
function Die  ([string]$m) { Write-Host "  [X]  $m"   -ForegroundColor Red; exit 1 }

Write-Host ""
Write-Host "=== Зогсоолын урсгал нийтлэгч — суулгалт ===" -ForegroundColor Cyan
Write-Host ""

# ── 0. Урьдчилсан шалгалт ────────────────────────────────────────────────
$admin = ([Security.Principal.WindowsPrincipal] `
    [Security.Principal.WindowsIdentity]::GetCurrent()
).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { Die "Админ эрхтэй PowerShell дээр ажиллуулна уу." }

if (-not (Test-Path $InstallDir)) { Die "Хавтас олдсонгүй: $InstallDir  (-InstallDir -ээр заана уу)" }

$ConfigPath = Join-Path $InstallDir "config.toml"
if (-not (Test-Path $ConfigPath)) { Die "config.toml олдсонгүй: $ConfigPath" }

# Нууц үгэнд `@` эсвэл `/` байвал URL задардаг — эрт барих нь дээр.
if ($MediamtxUrl -notmatch '^rtsps?://[^:/@]+:[^@/]+@[^/@]+:\d+/?$') {
    Warn "MediamtxUrl хэлбэр сэжигтэй байна: $($MediamtxUrl -replace ':[^:@]+@', ':***@')"
    Warn "Хүлээгдэж буй хэлбэр: rtsp://publisher:НУУЦҮГ@103.236.194.99:8554"
    Warn "Нууц үгэнд @ : / # тэмдэг байвал percent-encode хийнэ үү (@ -> %40)."
}

# ── 1. Үйлчилгээг олж зогсоох ────────────────────────────────────────────
if (-not $ServiceName) {
    $svc = Get-Service | Where-Object { $_.Name -like "*dahua*" -or $_.DisplayName -like "*dahua*" } |
           Select-Object -First 1
    if ($svc) { $ServiceName = $svc.Name }
}

$svcBaisan = $false
if ($ServiceName -and (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue)) {
    $s = Get-Service -Name $ServiceName
    Say "Үйлчилгээ: $ServiceName ($($s.Status))"
    if ($s.Status -eq "Running") {
        Stop-Service -Name $ServiceName -Force
        (Get-Service $ServiceName).WaitForStatus("Stopped", "00:00:30")
        $svcBaisan = $true
        Ok "Зогслоо"
    }
} else {
    Warn "Windows үйлчилгээ олдсонгүй — гараар ажилладаг бол дуусаад өөрөө дахин асаана уу."
}

# Файл түгжээтэй үлдэх тохиолдол байдаг.
Get-Process -Name "dahua-service" -ErrorAction SilentlyContinue | ForEach-Object {
    Warn "dahua-service процесс (pid $($_.Id)) хаагдаж байна"
    $_ | Stop-Process -Force
    Start-Sleep -Milliseconds 800
}

# ── 2. Нөөцлөх ───────────────────────────────────────────────────────────
$stamp  = Get-Date -Format "yyyyMMdd-HHmmss"
$backup = Join-Path $InstallDir "backup-$stamp"
New-Item -ItemType Directory -Path $backup -Force | Out-Null
Copy-Item $ConfigPath (Join-Path $backup "config.toml") -Force
if (Test-Path (Join-Path $InstallDir "dahua-service.exe")) {
    Copy-Item (Join-Path $InstallDir "dahua-service.exe") (Join-Path $backup "dahua-service.exe") -Force
}
Ok "Нөөц: $backup"

# ── 3. Файл хуулах ───────────────────────────────────────────────────────
if (-not $ConfigOnly) {
    $payload = Join-Path $ScriptDir "payload"
    if (-not (Test-Path $payload)) { Die "payload хавтас олдсонгүй: $payload" }

    # ffmpeg — үйлчилгээний хавтаст тавина, PATH-аас хамаарахгүй.
    $ffSrc = Join-Path $payload "ffmpeg.exe"
    if (Test-Path $ffSrc) {
        Copy-Item $ffSrc (Join-Path $InstallDir "ffmpeg.exe") -Force
        Ok "ffmpeg.exe хуулагдлаа"
    } else {
        Warn "payload дотор ffmpeg.exe алга — PATH дээрхийг ашиглана"
    }

    $exeSrc = Join-Path $payload "dahua-service.exe"
    if (Test-Path $exeSrc) {
        Copy-Item $exeSrc (Join-Path $InstallDir "dahua-service.exe") -Force
        Ok "dahua-service.exe шинэчлэгдлээ"
    } else {
        Warn "payload дотор dahua-service.exe алга — хуучин бинар үлдэнэ"
    }

    # Dahua SDK — шинэ компьютер дээр л хэрэгтэй, байгаа дээр нь бичихгүй.
    foreach ($dll in @("dhnetsdk.dll", "dhconfigsdk.dll", "avnetsdk.dll")) {
        $src = Join-Path $payload $dll
        $dst = Join-Path $InstallDir $dll
        if ((Test-Path $src) -and -not (Test-Path $dst)) {
            Copy-Item $src $dst -Force
            Ok "$dll хуулагдлаа"
        }
    }
}

# ── 4. config.toml — [publish] хэсэг ─────────────────────────────────────
if (-not $FilesOnly) {
    $text = Get-Content $ConfigPath -Raw -Encoding UTF8

    # Хуучин [publish] хэсгийг бүхэлд нь авна: дараагийн хүснэгт эхлэх
    # хүртэл. Ингэснээр дахин ажиллуулахад давхардахгүй.
    $text = [regex]::Replace(
        $text,
        '(?ms)^\[publish\]\s*.*?(?=^\s*\[|\z)',
        '',
        [Text.RegularExpressions.RegexOptions]::Multiline
    )
    $text = $text.TrimEnd()

    function Toml-Str([string]$v) { '"' + ($v -replace '\\', '\\\\' -replace '"', '\"') + '"' }

    $lines = @()
    $lines += ""
    $lines += "# Камерын урсгалыг VPS дээрх MediaMTX рүү тасралтгүй нийтэлнэ."
    $lines += "# install.ps1 -ээр $(Get-Date -Format 'yyyy-MM-dd HH:mm') -д бичигдсэн."
    $lines += "[publish]"
    $lines += "url = $(Toml-Str $MediamtxUrl)"

    $ffLocal = Join-Path $InstallDir "ffmpeg.exe"
    if (Test-Path $ffLocal) { $lines += "ffmpeg = $(Toml-Str $ffLocal)" }

    if ($SkipIps.Count -gt 0) {
        $joined = ($SkipIps | ForEach-Object { Toml-Str $_ }) -join ", "
        $lines += "skip_ips = [$joined]"
    }

    $out = $text + "`r`n" + ($lines -join "`r`n") + "`r`n"
    # BOM-гүй UTF-8: Rust-ын toml задлагч BOM-той ч ажилладаг ч цэвэр нь дээр.
    [IO.File]::WriteAllText($ConfigPath, $out, (New-Object Text.UTF8Encoding($false)))
    Ok "config.toml -д [publish] бичигдлээ"
}

# ── 5. Асаах ─────────────────────────────────────────────────────────────
if ($ServiceName -and (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue)) {
    Start-Service -Name $ServiceName
    (Get-Service $ServiceName).WaitForStatus("Running", "00:00:30")
    Ok "Үйлчилгээ асаалаа"
} elseif ($svcBaisan) {
    Warn "Үйлчилгээг гараар асаана уу"
} else {
    Say "Гараар турших:  cd `"$InstallDir`"; .\dahua-service.exe run"
}

# ── 6. Баталгаажуулалт ───────────────────────────────────────────────────
$log = Join-Path $InstallDir "service.log"
if (Test-Path $log) {
    Say "Лог ажиглаж байна (20 секунд)..."
    $tugsgul = (Get-Date).AddSeconds(20)
    $olson = $false
    while ((Get-Date) -lt $tugsgul) {
        Start-Sleep -Seconds 2
        $sul = Get-Content $log -Tail 80 -ErrorAction SilentlyContinue
        $mur = $sul | Where-Object { $_ -match "PUBLISH \|" }
        if ($mur) {
            $olson = $true
            $mur | Select-Object -Last 8 | ForEach-Object { Say $_ }
            break
        }
    }
    if ($olson) { Ok "Нийтлэгч ажиллаж эхэллээ" }
    else { Warn "PUBLISH мөр олдсонгүй. Шалгах:  Get-Content `"$log`" -Tail 60" }
}

Write-Host ""
Write-Host "Дуусав." -ForegroundColor Cyan
Write-Host "VPS дээр шалгах:  curl -s http://127.0.0.1:9997/v3/paths/list" -ForegroundColor DarkGray
Write-Host ""
