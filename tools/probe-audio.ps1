<#
    probe-audio.ps1 — камерын дуут мэдэгдлийн боломжийг шалгана (READ-ONLY).

    Камер бүр дээр ЗӨВХӨН GET хүсэлт явуулж, тохиргоог уншина. Юу ч бичихгүй,
    юу ч өөрчлөхгүй. Нууц үгийг config.toml-с уншина — гараар оруулах шаардлагагүй.

    Ажиллуулах (dahua-service.exe байрлах фолдерт):
        powershell -ExecutionPolicy Bypass -File tools\probe-audio.ps1

    Үр дүн: probe-audio-output.txt
#>

param(
    [string]$ConfigPath = "",
    [string]$Ip         = "",
    [string]$OutFile    = ""
)

$ErrorActionPreference = "Continue"

# ── config.toml олох ────────────────────────────────────────────────────────
if (-not $ConfigPath) {
    $here = Split-Path -Parent $MyInvocation.MyCommand.Path
    $candidates = @(
        (Join-Path $here "config.toml"),
        (Join-Path (Split-Path -Parent $here) "config.toml")
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { $ConfigPath = $c; break }
    }
}
if (-not $ConfigPath -or -not (Test-Path $ConfigPath)) {
    Write-Host "config.toml олдсонгүй. -ConfigPath <зам> гэж заана уу." -ForegroundColor Red
    exit 1
}
if (-not $OutFile) {
    $OutFile = Join-Path (Split-Path -Parent $ConfigPath) "probe-audio-output.txt"
}

# ── [[cameras]] блокуудыг задлах ────────────────────────────────────────────
$cameras = @()
$current = $null
foreach ($line in (Get-Content $ConfigPath)) {
    $t = $line.Trim()
    if ($t -match '^\[\[cameras\]\]') {
        if ($current) { $cameras += $current }
        $current = @{ ip = ""; password = ""; http_port = 443 }
        continue
    }
    if ($t -match '^\[') {
        if ($current) { $cameras += $current; $current = $null }
        continue
    }
    if (-not $current) { continue }
    if ($t -match '^ip\s*=\s*"([^"]*)"')       { $current.ip       = $Matches[1] }
    if ($t -match '^password\s*=\s*"([^"]*)"') { $current.password = $Matches[1] }
    if ($t -match '^http_port\s*=\s*(\d+)')    { $current.http_port = [int]$Matches[1] }
}
if ($current) { $cameras += $current }

if ($Ip) { $cameras = $cameras | Where-Object { $_.ip -eq $Ip } }
if (-not $cameras -or $cameras.Count -eq 0) {
    Write-Host "config.toml дотор камер олдсонгүй." -ForegroundColor Red
    exit 1
}

# ── TLS / гэрчилгээний шалгалтыг тойрох (камер self-signed) ─────────────────
$isPs7 = $PSVersionTable.PSVersion.Major -ge 6
if (-not $isPs7) {
    [System.Net.ServicePointManager]::ServerCertificateValidationCallback = { $true }
    try {
        [System.Net.ServicePointManager]::SecurityProtocol =
            [System.Net.SecurityProtocolType]::Tls12 -bor
            [System.Net.SecurityProtocolType]::Tls11 -bor
            [System.Net.SecurityProtocolType]::Tls
    } catch {}
}

function Test-TcpPort([string]$hostIp, [int]$port) {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $async = $client.BeginConnect($hostIp, $port, $null, $null)
        $ok = $async.AsyncWaitHandle.WaitOne(800, $false)
        if ($ok -and $client.Connected) { return $true }
        return $false
    } catch {
        return $false
    } finally {
        $client.Close()
    }
}

# probe.rs-тэй ижил логик: preferred эхэлж, хаалттай бол нөгөөг нь
function Resolve-CameraPort([string]$hostIp, [int]$preferred) {
    if (Test-TcpPort $hostIp $preferred) { return $preferred }
    if ($preferred -eq 443) { $other = 80 } else { $other = 443 }
    if (Test-TcpPort $hostIp $other) { return $other }
    return $preferred
}

function Invoke-CameraGet([string]$url, [System.Management.Automation.PSCredential]$cred) {
    try {
        if ($isPs7) {
            $r = Invoke-WebRequest -Uri $url -Credential $cred -SkipCertificateCheck -TimeoutSec 10 -UseBasicParsing
        } else {
            $r = Invoke-WebRequest -Uri $url -Credential $cred -TimeoutSec 10 -UseBasicParsing
        }
        return @{ ok = $true; status = [int]$r.StatusCode; body = $r.Content }
    } catch {
        $status = 0
        if ($_.Exception.Response) {
            try { $status = [int]$_.Exception.Response.StatusCode } catch {}
        }
        return @{ ok = $false; status = $status; body = $_.Exception.Message }
    }
}

$report = New-Object System.Collections.Generic.List[string]
function Say([string]$text) {
    Write-Host $text
    $report.Add($text)
}

Say "=== Dahua дуут мэдэгдлийн боломжийн шалгалт ==="
Say "Огноо: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Say "Config: $ConfigPath"
Say "PowerShell: $($PSVersionTable.PSVersion)"
Say ""

# Шалгах тохиргооны хэсгүүд. Бүгд GET — юу ч бичихгүй.
$configNames = @(
    "TrafficLatticeScreen",
    "TrafficSnapVoice",
    "VoiceBroadcast",
    "AudioOutput",
    "AudioInput",
    "TrafficGlobal",
    "TrafficSnap"
)

# Нэмэлт CGI цэгүүд — байгаа эсэхийг л шалгана
$extraEndpoints = @(
    "/cgi-bin/magicBox.cgi?action=getDeviceType",
    "/cgi-bin/magicBox.cgi?action=getSoftwareVersion",
    "/cgi-bin/magicBox.cgi?action=getProductDefinition",
    "/cgi-bin/audio.cgi?action=getCaps",
    "/cgi-bin/audioFile.cgi?action=getCollect",
    "/cgi-bin/devAudioOutput.cgi?action=getCollect"
)

foreach ($cam in $cameras) {
    $camIp = $cam.ip
    Say "────────────────────────────────────────────────────────"
    Say "КАМЕР: $camIp"

    $port = Resolve-CameraPort $camIp $cam.http_port
    if ($port -eq 443) { $scheme = "https" } else { $scheme = "http" }
    if (($port -eq 443) -or ($port -eq 80)) { $hostPart = $camIp } else { $hostPart = "${camIp}:${port}" }
    $base = "${scheme}://${hostPart}"
    Say "Порт: $port ($base)"

    $sec  = ConvertTo-SecureString $cam.password -AsPlainText -Force
    $cred = New-Object System.Management.Automation.PSCredential("admin", $sec)

    foreach ($ep in $extraEndpoints) {
        $res  = Invoke-CameraGet "$base$ep" $cred
        $head = ""
        if ($res.body) {
            $lines = ($res.body -split "`r?`n") | Where-Object { $_.Trim() -ne "" }
            $head  = ($lines | Select-Object -First 4) -join " | "
        }
        Say ("  [{0}] {1}" -f $res.status, $ep)
        if ($head) { Say "        $head" }
    }

    foreach ($name in $configNames) {
        $res = Invoke-CameraGet "$base/cgi-bin/configManager.cgi?action=getConfig&name=$name" $cred
        Say ""
        Say "  --- getConfig name=$name  (HTTP $($res.status)) ---"
        if ($res.ok -and $res.body) {
            foreach ($l in ($res.body -split "`r?`n")) {
                if ($l.Trim() -ne "") { Say "    $l" }
            }
        } else {
            Say "    (байхгүй эсвэл алдаа) $($res.body)"
        }
    }

    # Бүх тохиргооноос audio/voice/play гэсэн мөрүүдийг шүүнэ — firmware бүр
    # өөр нэр ашигладаг тул жинхэнэ талбарын нэр эндээс гарна.
    Say ""
    Say "  --- name=All дотроос audio/voice/play/speak/broadcast мөрүүд ---"
    $all = Invoke-CameraGet "$base/cgi-bin/configManager.cgi?action=getConfig&name=All" $cred
    if ($all.ok -and $all.body) {
        $allLines = $all.body -split "`r?`n"
        $hits = $allLines | Where-Object { $_ -match '(?i)audio|voice|play|speak|broadcast|horn|sound' }
        if ($hits) {
            foreach ($h in $hits) { Say "    $h" }
        } else {
            Say "    (тохирох мөр олдсонгүй)"
        }
        Say "    [name=All нийт мөр: $($allLines.Count)]"
    } else {
        Say "    (уншиж чадсангүй) HTTP $($all.status) $($all.body)"
    }
    Say ""
}

Say "────────────────────────────────────────────────────────"
$report | Out-File -FilePath $OutFile -Encoding utf8
Write-Host ""
Write-Host "Үр дүн хадгалагдлаа: $OutFile" -ForegroundColor Green
