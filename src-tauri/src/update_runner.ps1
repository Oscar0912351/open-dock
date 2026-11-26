$ErrorActionPreference = 'SilentlyContinue'
$installer = '__INSTALLER__'
$relaunch  = '__RELAUNCH__'
$splashPs1 = '__SPLASH_SCRIPT__'
$stageFile = '__STAGE_FILE__'
$runDir   = '__RUN_DIR__'
$appPid    = __APP_PID__

function Set-Stage([string]$name) {
    try { [System.IO.File]::WriteAllText($stageFile, $name) } catch {}
}

try { Wait-Process -Id $appPid -Timeout 15 } catch {}
Start-Sleep -Milliseconds 450

Set-Stage 'installing'
$iproc = $null
try { $iproc = Start-Process -FilePath $installer -ArgumentList '/S' -PassThru } catch {}
if ($iproc) {
    # En hængt installer må aldrig efterlade statusboksen evigt — dræb efter 3 min.
    try { if (-not $iproc.WaitForExit(180000)) { Stop-Process -Id $iproc.Id -Force } } catch {}
}

Set-Stage 'relaunching'
Start-Sleep -Milliseconds 250
try { Start-Process -FilePath $relaunch } catch {}

Set-Stage 'done'
# Splashen lukker sig selv på 'done' (plus 3 min selvdestruktion som net) —
# giv dens 180 ms-poll tid til at nå at læse stadiet før filen fjernes.
Start-Sleep -Milliseconds 800
try { Remove-Item -LiteralPath $stageFile -Force } catch {}
try { Remove-Item -LiteralPath $splashPs1 -Force } catch {}
try { Remove-Item -LiteralPath $PSCommandPath -Force } catch {}
try { Remove-Item -LiteralPath $runDir -Force } catch {}
