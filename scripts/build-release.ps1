# Choose a version, commit project changes, build, push the branch, then publish.
[CmdletBinding(DefaultParameterSetName='Choose')]
param(
    [Parameter(ParameterSetName='Choose')][string]$Version,
    [Parameter(ParameterSetName='Increment')][ValidateSet('1','2','3')][string]$Increment,
    [Parameter(ParameterSetName='BuildOnly',Mandatory)][switch]$BuildOnly,
    [Parameter(ParameterSetName='Resume',Mandatory)][switch]$Resume,
    [string]$MakeNsis
)

$ErrorActionPreference = 'Stop'
function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program $($Arguments[0]) failed (exit $LASTEXITCODE)." }
}

Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    if (-not $BuildOnly) {
        Get-Command gh -CommandType Application -ErrorAction Stop | Out-Null
        $root = Invoke-Checked git @('rev-parse', '--show-toplevel')
        if ([IO.Path]::GetFullPath($root) -ne (Get-Location).Path) { throw 'Run from the application repository.' }
        $branch = Invoke-Checked git @('symbolic-ref', '--short', 'HEAD')
        if (Invoke-Checked git @('diff', '--name-only', '--diff-filter=U')) { throw 'Resolve Git conflicts before releasing.' }
        if ($Resume) {
            if (Invoke-Checked git @('status', '--porcelain', '--untracked-files=normal')) {
                throw 'Commit any pending changes before resuming a release.'
            }
        } else {
            $meta = (Invoke-Checked cargo @('metadata', '--no-deps', '--format-version', '1', '--locked')) | ConvertFrom-Json
            $current = [string]($meta.packages | Where-Object name -eq 'noia-dnf').version
            $v = [version]$current
            $choices = @{
                '1' = "$($v.Major).$($v.Minor).$($v.Build + 1)"
                '2' = "$($v.Major).$($v.Minor + 1).0"
                '3' = "$($v.Major + 1).0.0"
            }
            if ($Increment) { $Version = $choices[$Increment] }
            if (-not $Version) {
                Write-Host "Current: $current  |  1 Patch: $($choices['1'])  |  2 Minor: $($choices['2'])  |  3 Major: $($choices['3'])"
                $answer = Read-Host 'Choose 1-3 or enter a version (Enter cancels)'
                if (-not $answer) { return }
                $Version = if ($choices.ContainsKey($answer)) { $choices[$answer] } else { $answer.Trim() }
            }
            if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' -or [version]$Version -le $v) {
                throw "Choose a stable x.y.z version newer than $current."
            }
            & (Join-Path $PSScriptRoot 'read-changelog.ps1') -Version $Version | Out-Null
            # Edit only the app's version in the two Cargo files. Preserve other packages and formatting.
            $edits = foreach ($name in 'Cargo.toml', 'Cargo.lock') {
                $path = (Resolve-Path $name).Path
                $content = [IO.File]::ReadAllText($path)
                $section = if ($name -eq 'Cargo.toml') { '\[package\]' } else { '\[\[package\]\]' }
                $pattern = '(?m)^' + $section + '\r?\nname = "noia-dnf"\r?\nversion = "(?<version>' + [regex]::Escape($current) + ')"'
                $hits = [regex]::Matches($content, $pattern)
                if ($hits.Count -ne 1) { throw "Cannot locate the noia-dnf $current package in $name." }
                $index = $hits[0].Groups['version'].Index
                @{ Path = $path; Text = $content.Remove($index,$current.Length).Insert($index,$Version) }
            }
            foreach ($edit in $edits) { [IO.File]::WriteAllText($edit.Path, $edit.Text, [Text.UTF8Encoding]::new($false)) }
            Invoke-Checked git @('add', '--all', '--', '.') | Out-Host
            Invoke-Checked git @('commit', '-m', "Release v$Version") | Out-Host
            Write-Host "Created release commit for v$Version. To retry this version use -Resume."
        }
        $commit = Invoke-Checked git @('rev-parse', 'HEAD')
        if (Invoke-Checked git @('status', '--porcelain', '--untracked-files=normal')) {
            throw 'Source changed during the commit. Resolve the changes and run with -Resume.'
        }
    }

    Invoke-Checked cargo @('test', '--workspace', '--release', '--locked') | Out-Host
    & (Join-Path $PSScriptRoot 'build-installer.ps1') -MakeNsis $MakeNsis | Out-Host
    $metadata = (Invoke-Checked cargo @('metadata', '--no-deps', '--format-version', '1', '--locked')) | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'noia-dnf').version
    $tag = "v$version"
    $manifestFile = Get-Item -LiteralPath (Join-Path $metadata.target_directory 'installer/latest.json')
    $manifest = Get-Content -Raw -Encoding UTF8 -LiteralPath $manifestFile.FullName | ConvertFrom-Json
    $asset = $manifest.assets.'windows-x86_64'
    $installer = Get-Item -LiteralPath (Join-Path $manifestFile.DirectoryName "Noia-DNF-$version-windows-x64-setup.exe")
    $zip = Get-Item -LiteralPath (Join-Path $metadata.target_directory 'Noia-DNF-windows-x64.zip')
    $files = @($installer, $manifestFile, $zip)
    if ($manifest.version -cne $version -or $asset.size -ne $installer.Length -or
        $asset.sha256 -ne (Get-FileHash -LiteralPath $installer.FullName).Hash) {
        throw 'The packaged installer does not match latest.json.'
    }
    $suffix = [regex]::Escape("/releases/download/$tag/$($installer.Name)")
    if ($asset.url -cnotmatch "^https://github\.com/(?<repo>[\w.-]+/[\w.-]+)$suffix$") {
        throw 'Unexpected GitHub release URL in latest.json.'
    }
    $repo = $Matches.repo
    if ($BuildOnly) { Write-Host "Verified local release $repo $tag."; return }

    if ((Invoke-Checked git @('rev-parse', 'HEAD')) -ne $commit -or
        (Invoke-Checked git @('symbolic-ref', '--short', 'HEAD')) -ne $branch -or
        (Invoke-Checked git @('status', '--porcelain', '--untracked-files=normal'))) {
        throw 'Source changed during the build. Rebuild from the release commit.'
    }
    Invoke-Checked git @('push', 'origin', "${commit}:refs/heads/$branch") | Out-Host
    $parts = $repo.Split('/')
    $query = 'query($owner:String!,$name:String!,$sha:String!,$ref:String!,$tag:String!){repository(owner:$owner,name:$name){object(expression:$sha){oid} ref(qualifiedName:$ref){name} release(tagName:$tag){isDraft url}}}'
    $remote = ((Invoke-Checked gh @('api', 'graphql', '-f', "query=$query", '-f', "owner=$($parts[0])",
        '-f', "name=$($parts[1])", '-f', "sha=$commit", '-f', "ref=refs/tags/$tag", '-f', "tag=$tag")) | ConvertFrom-Json).data.repository
    if ($remote.object.oid -ne $commit) { throw "Push release commit $commit to $repo first." }
    if ($remote.release -and -not $remote.release.isDraft) {
        throw "Already published: $($remote.release.url). Increment the version for the next release."
    }
    if (-not $remote.ref) {
        Invoke-Checked gh @('api', '--method', 'POST', "repos/$repo/git/refs", '-f', "ref=refs/tags/$tag", '-f', "sha=$commit") | Out-Null
    }
    # The commits endpoint also resolves annotated tags.
    $tagCommit = Invoke-Checked gh @('api', "repos/$repo/commits/$tag", '--jq', '.sha')
    if ($tagCommit -ne $commit) { throw "Tag $tag points to $tagCommit, not the release commit $commit." }

    $notesFile = New-TemporaryFile
    try {
        $lines = @($manifest.changelog.items | ForEach-Object { "- $($_.'zh-CN')" })
        $lines += @('', '## English', '')
        $lines += @($manifest.changelog.items | ForEach-Object { "- $($_.en)" })
        [IO.File]::WriteAllLines($notesFile.FullName, [string[]]$lines, [Text.UTF8Encoding]::new($false))
        if (-not $remote.release) {
            Invoke-Checked gh @('release', 'create', $tag, '--repo', $repo, '--verify-tag', '--draft',
                '--title', "Noia DNF $tag", '--notes-file', $notesFile.FullName) | Out-Host
        }
        $draft = (Invoke-Checked gh @('release', 'view', $tag, '--repo', $repo, '--json', 'databaseId,isDraft')) | ConvertFrom-Json
        if (-not $draft.isDraft) { throw "$tag is no longer a draft." }
        Invoke-Checked gh (@('release', 'upload', $tag, '--repo', $repo, '--clobber') + $files.FullName) | Out-Host
        $assets = (Invoke-Checked gh @('api', "repos/$repo/releases/$($draft.databaseId)/assets?per_page=100")) | ConvertFrom-Json
        foreach ($file in $files) {
            $uploaded = @($assets | Where-Object name -ceq $file.Name)
            $hash = (Get-FileHash -LiteralPath $file.FullName).Hash
            if ($uploaded.Count -ne 1 -or $uploaded[0].size -ne $file.Length -or $uploaded[0].digest -ne "sha256:$hash") {
                throw "GitHub asset verification failed for $($file.Name). Rerun to resume the draft."
            }
        }
        Invoke-Checked gh @('release', 'edit', $tag, '--repo', $repo, '--verify-tag', '--draft=false', '--latest',
            '--notes-file', $notesFile.FullName) | Out-Host
    } finally {
        Remove-Item -LiteralPath $notesFile.FullName
    }
    $latest = Invoke-Checked gh @('api', "repos/$repo/releases/latest", '--jq', '.id')
    if ($latest -ne [string]$draft.databaseId) { throw "Published $tag, but GitHub has a different Latest release." }
    Write-Host "Published https://github.com/$repo/releases/tag/$tag"
    Write-Host "Updater manifest: https://github.com/$repo/releases/latest/download/latest.json"
} finally {
    Pop-Location
}
