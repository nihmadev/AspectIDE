import { execFileSync } from "node:child_process";

const port = Number(process.argv[2] ?? 5173);

if (!Number.isInteger(port) || port < 1 || port > 65535) {
  console.error(`Invalid port: ${process.argv[2] ?? ""}`);
  process.exit(1);
}

if (process.platform !== "win32") process.exit(0);

const script = `
$port = ${port}
$currentPid = $PID
$processIds = @()
$connections = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
if ($connections) {
  $processIds += $connections | Select-Object -ExpandProperty OwningProcess -Unique
}

$netstatRows = netstat -ano -p tcp | Select-String -Pattern (":" + $port + "\\s")
foreach ($row in $netstatRows) {
  $parts = ($row.Line -replace "^\\s+", "") -split "\\s+"
  if ($parts.Length -ge 5 -and $parts[1] -match (":" + $port + "$") -and $parts[3] -eq "LISTENING") {
    $processIds += [int]$parts[4]
  }
}

$processIds = $processIds | Select-Object -Unique | Where-Object { $_ -and $_ -ne $currentPid }
foreach ($processId in $processIds) {
  try {
    Stop-Process -Id $processId -Force -ErrorAction Stop
    Write-Output ("Killed process {0} on port {1}" -f $processId, $port)
  } catch {
    Write-Error ("Failed to kill process {0} on port {1}: {2}" -f $processId, $port, $_.Exception.Message)
    exit 1
  }
}

for ($attempt = 0; $attempt -lt 20; $attempt++) {
  $busy = netstat -ano -p tcp | Select-String -Pattern (":" + $port + "\\s") | Where-Object { $_.Line -match "LISTENING" }
  if (-not $busy) { exit 0 }
  Start-Sleep -Milliseconds 150
}

Write-Error ("Port {0} is still busy after kill attempt" -f $port)
exit 1
`;

execFileSync("powershell.exe", ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script], {
  stdio: "inherit",
});
