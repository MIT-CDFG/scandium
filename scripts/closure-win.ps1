# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 the Appname contributors
# D-13 static import closure (O-WIN): every import of every PE in the staged folder resolves to
# the zip, an API set, a System32 DLL signed "Microsoft Windows", or the VC++ Redistributable (RR-W1).
#   powershell -File scripts/closure-win.ps1 -Stage <staged Scandium folder> -Dumpbin <dumpbin.exe>
param([string]$Stage, [string]$Dumpbin)
$pe = Get-ChildItem -Recurse -File $Stage | Where-Object { $_.Extension -in '.exe','.dll' }
$inZip = @{}; foreach ($f in $pe) { $inZip[$f.Name.ToLower()] = $true }
$redist = '^(vcruntime140(_1)?|msvcp140(_1|_2|_atomic_wait|_codecvt_ids)?|concrt140|vccorlib140)\.dll$'
$rows = @{}
foreach ($f in $pe) {
  $deps = & $Dumpbin /nologo /dependents $f.FullName | ForEach-Object { $_.Trim() } | Where-Object { $_ -match '\.dll$' -and $_ -notmatch '^Dump of' }
  foreach ($d in $deps) {
    $k = $d.ToLower()
    if ($inZip.ContainsKey($k)) { $c = 'zip' }
    elseif ($k -match '^(api|ext)-ms-') { $c = 'apiset' }
    elseif ($k -match $redist) { $c = 'redist' }
    else {
      $sys = Join-Path $env:WINDIR "System32\$d"
      if (Test-Path $sys) {
        $sig = Get-AuthenticodeSignature $sys
        if ($sig.Status -eq 'Valid' -and $sig.SignerCertificate.Subject -match '^CN=Microsoft Windows,') { $c = 'os' }
        else { $c = "UNRESOLVED(System32, signer=$($sig.SignerCertificate.Subject), status=$($sig.Status))" }
      } else { $c = 'UNRESOLVED' }
    }
    if (-not $rows.ContainsKey($k)) { $rows[$k] = @{ class = $c; by = New-Object System.Collections.Generic.List[string] } }
    $rows[$k].by.Add($f.Name)
  }
}
"PE files scanned: $($pe.Count)"
foreach ($cls in 'zip','apiset','os','redist') { $n = @($rows.Keys | Where-Object { $rows[$_].class -eq $cls }); "${cls}: $($n.Count)" + $(if ($cls -in 'os','redist') { ' - ' + (($n | Sort-Object) -join ' ') } else { '' }) }
$bad = @($rows.Keys | Where-Object { $rows[$_].class -like 'UNRESOLVED*' })
foreach ($b in $bad) { "UNRESOLVED $b [$($rows[$b].class)] imported by: $((($rows[$b].by | Sort-Object -Unique) | Select-Object -First 6) -join ', ')" }
"CLOSURE " + $(if ($bad.Count -eq 0) { 'CLEAN' } else { "RED ($($bad.Count))" })
