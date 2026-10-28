# Parser errors and PSScriptAnalyzer findings of one script; exit code = their count.
param([Parameter(Mandatory)] [string] $Path)

$tokens = $null
$errors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile($Path, [ref]$tokens, [ref]$errors)
$errors | ForEach-Object {
    Write-Output ('{0}:{1}: {2}' -f $_.Extent.StartLineNumber, $_.Extent.StartColumnNumber, $_.Message)
}
Write-Output "parser errors: $(@($errors).Count)"

$found = @(Invoke-ScriptAnalyzer -Path $Path -Severity Information, Warning, Error)
$found | ForEach-Object { Write-Output ('{0} {1}:{2} {3}' -f $_.Severity, $_.RuleName, $_.Line, $_.Message) }
$version = (Get-Module -ListAvailable PSScriptAnalyzer | Select-Object -First 1).Version
Write-Output "PSScriptAnalyzer $version findings: $($found.Count)"
exit (@($errors).Count + $found.Count)
