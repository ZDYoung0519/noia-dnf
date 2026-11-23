param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
$history = Get-Content -Raw -Encoding UTF8 -LiteralPath (Join-Path $PSScriptRoot '../change_log.json') | ConvertFrom-Json
$entries = @($history.releases | Where-Object version -ceq $Version)
if ($entries.Count -ne 1) { throw "Write exactly one entry for $Version in change_log.json before releasing." }
$notes = $entries[0]
if ($notes.items -isnot [array] -or $notes.items.Count -lt 1) { throw "Missing changelog items for $Version." }
foreach ($item in $notes.items) {
    foreach ($lang in 'zh-CN', 'en') {
        if ($item.$lang -isnot [string] -or [string]::IsNullOrWhiteSpace($item.$lang)) {
            throw "Missing $lang text in change_log.json for $Version."
        }
    }
}
$notes
