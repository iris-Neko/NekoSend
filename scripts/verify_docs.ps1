#Requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$docRoot = Join-Path $repoRoot 'docs'
$markdownFiles = @(
    Join-Path $repoRoot 'README.md'
    Get-ChildItem -LiteralPath $docRoot -Filter '*.md' -File |
        Sort-Object Name |
        Select-Object -ExpandProperty FullName
)
$failures = [System.Collections.Generic.List[string]]::new()
$checks = [System.Collections.Generic.List[string]]::new()

function Add-Failure([string]$Message) {
    $failures.Add($Message)
}

function Normalize-Text([string]$Value) {
    return (($Value -replace "`r`n", "`n") -replace "`r", "`n").Trim()
}

function Get-GitHubAnchor([string]$Heading) {
    $anchor = $Heading.ToLowerInvariant()
    $anchor = $anchor -replace '[`*_~]', ''
    $anchor = $anchor -replace '[^\p{L}\p{Nd}\s_-]', ''
    $anchor = $anchor -replace '\s+', '-'
    return $anchor.Trim('-')
}

function Get-HeadingAnchors([string]$Content) {
    $anchors = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $counts = @{}
    foreach ($match in [regex]::Matches($Content, '(?m)^#{1,6}\s+(?<title>.+?)\s*$')) {
        $base = Get-GitHubAnchor $match.Groups['title'].Value
        if ([string]::IsNullOrWhiteSpace($base)) { continue }
        $count = if ($counts.ContainsKey($base)) { [int]$counts[$base] } else { 0 }
        $anchor = if ($count -eq 0) { $base } else { "$base-$count" }
        $counts[$base] = $count + 1
        [void]$anchors.Add($anchor)
    }
    return $anchors
}

function Expand-IdReference([string]$Reference) {
    $result = [System.Collections.Generic.List[string]]::new()
    $lastPrefix = $null
    foreach ($part in $Reference.Split(',')) {
        $value = $part.Trim()
        if ($value -match '^(?<prefix>[A-Z0-9]+(?:-[A-Z0-9]+)*)-(?<first>\d+)\.\.(?<last>\d+)$') {
            $lastPrefix = $Matches['prefix']
            $width = $Matches['first'].Length
            for ($number = [int]$Matches['first']; $number -le [int]$Matches['last']; $number++) {
                $result.Add(('{0}-{1}' -f $Matches['prefix'], $number.ToString("D$width")))
            }
        } elseif ($null -ne $lastPrefix -and $value -match '^(?<first>\d+)\.\.(?<last>\d+)$') {
            $width = $Matches['first'].Length
            for ($number = [int]$Matches['first']; $number -le [int]$Matches['last']; $number++) {
                $result.Add(('{0}-{1}' -f $lastPrefix, $number.ToString("D$width")))
            }
        } elseif ($value -match '^(?<prefix>[A-Z0-9]+(?:-[A-Z0-9]+)*)-\d+$') {
            $lastPrefix = $Matches['prefix']
            $result.Add($value)
        } else {
            Add-Failure "Invalid ID reference: $value"
        }
    }
    return $result
}

# DOC-001: local links and heading anchors.
$contentByPath = @{}
$anchorsByPath = @{}
foreach ($file in $markdownFiles) {
    $resolved = (Resolve-Path -LiteralPath $file).Path
    $content = Get-Content -LiteralPath $resolved -Raw
    $contentByPath[$resolved] = $content
    $anchorsByPath[$resolved] = Get-HeadingAnchors $content
}

foreach ($file in $markdownFiles) {
    $resolved = (Resolve-Path -LiteralPath $file).Path
    $content = $contentByPath[$resolved]
    foreach ($match in [regex]::Matches($content, '(?<!!)\[[^\]]+\]\((?<target>[^)\s]+)')) {
        $target = $match.Groups['target'].Value.Trim('<', '>')
        if ($target -match '^(?:https?://|mailto:)') { continue }
        $parts = $target.Split('#', 2)
        $pathPart = [Uri]::UnescapeDataString($parts[0])
        $fragment = if ($parts.Count -eq 2) {
            [Uri]::UnescapeDataString($parts[1]).ToLowerInvariant()
        } else { '' }
        $targetPath = if ([string]::IsNullOrWhiteSpace($pathPart)) {
            $resolved
        } else {
            [IO.Path]::GetFullPath((Join-Path (Split-Path $resolved -Parent) $pathPart))
        }
        if (-not (Test-Path -LiteralPath $targetPath -PathType Leaf)) {
            Add-Failure "Broken link in $([IO.Path]::GetRelativePath($repoRoot, $resolved)): $target"
            continue
        }
        if (-not [string]::IsNullOrWhiteSpace($fragment)) {
            $targetResolved = (Resolve-Path -LiteralPath $targetPath).Path
            if (-not $anchorsByPath.ContainsKey($targetResolved)) {
                $targetContent = Get-Content -LiteralPath $targetResolved -Raw
                $anchorsByPath[$targetResolved] = Get-HeadingAnchors $targetContent
            }
            if (-not $anchorsByPath[$targetResolved].Contains($fragment)) {
                Add-Failure "Missing heading anchor in $([IO.Path]::GetRelativePath($repoRoot, $resolved)): $target"
            }
        }
    }
}
$checks.Add('DOC-001 local links and anchors')

# DOC-002: unfinished design markers in documentation.
$placeholderPattern = '(?im)\b(?:TODO|TBD|FIXME|XXX)\b|待定|待补充|后续补充'
foreach ($file in $markdownFiles) {
    foreach ($match in [regex]::Matches($contentByPath[(Resolve-Path $file).Path], $placeholderPattern)) {
        Add-Failure "Placeholder marker in $([IO.Path]::GetRelativePath($repoRoot, $file)): $($match.Value)"
    }
}
$checks.Add('DOC-002 placeholder markers')

# DOC-003: enum assignments have one authoritative document.
foreach ($file in $markdownFiles) {
    if ((Split-Path $file -Leaf) -eq '04-data-and-api.md') { continue }
    $content = $contentByPath[(Resolve-Path $file).Path]
    if ($content -match '(?m)^\w+(?:State|Kind|Mode|Policy|Decision|Direction|Reason|Role|Relation|Membership|Platform)\s*=') {
        Add-Failure "Enum assignment outside docs/04-data-and-api.md: $([IO.Path]::GetRelativePath($repoRoot, $file))"
    }
}
$authoritativeData = $contentByPath[(Resolve-Path (Join-Path $docRoot '04-data-and-api.md')).Path]
if ([regex]::Matches($authoritativeData, '(?m)^## 权威枚举\s*$').Count -ne 1) {
    Add-Failure 'docs/04-data-and-api.md must contain exactly one authoritative enum section.'
}
$checks.Add('DOC-003 authoritative enum location')

# DOC-004: every JSON block parses and contains the fields required by its message family.
$eventTypes = @(
    'text_message', 'file_offer', 'file_accept', 'file_reject',
    'transfer_pause', 'transfer_resume', 'group_invite', 'group_update',
    'own_device_bind', 'clipboard_update', 'delivery_receipt'
)
$jsonBlockCount = 0
foreach ($file in $markdownFiles) {
    $content = $contentByPath[(Resolve-Path $file).Path]
    foreach ($match in [regex]::Matches($content, '(?ms)^```json\s*\r?\n(?<json>.*?)\r?\n```')) {
        $jsonBlockCount++
        try {
            $message = $match.Groups['json'].Value | ConvertFrom-Json -ErrorAction Stop
        } catch {
            Add-Failure "Invalid JSON block in $([IO.Path]::GetRelativePath($repoRoot, $file)): $($_.Exception.Message)"
            continue
        }
        foreach ($field in @('version', 'type')) {
            if ($null -eq $message.PSObject.Properties[$field]) {
                Add-Failure "JSON type '$($message.type)' in $([IO.Path]::GetRelativePath($repoRoot, $file)) lacks '$field'."
            }
        }
        if ($eventTypes -contains $message.type) {
            foreach ($field in @('event_id', 'sender_device_id', 'sent_at_ms', 'body')) {
                if ($null -eq $message.PSObject.Properties[$field]) {
                    Add-Failure "Event '$($message.type)' lacks '$field'."
                }
            }
        }
        if ($message.type -in @('discover', 'announce', 'hello', 'hello_ack')) {
            if ($null -eq $message.PSObject.Properties['device_id'] -and
                $null -eq $message.PSObject.Properties['sender_device_id']) {
                Add-Failure "Handshake/discovery '$($message.type)' lacks a device identifier."
            }
        }
    }
}
if ($jsonBlockCount -lt 1) { Add-Failure 'No JSON examples were found.' }
$checks.Add("DOC-004 JSON examples ($jsonBlockCount blocks)")

# DOC-005: documented current DDL matches the result of all migrations.
$sqlBlocks = [regex]::Matches($authoritativeData, '(?ms)^```sql\s*\r?\n(?<sql>.*?)\r?\n```')
if ($sqlBlocks.Count -lt 2) {
        Add-Failure 'The documented current DDL block is missing.'
} else {
    $documentedDdl = Normalize-Text $sqlBlocks[1].Groups['sql'].Value
    $sqlite = Get-Command sqlite3 -ErrorAction SilentlyContinue
    if ($null -eq $sqlite) {
        Add-Failure 'sqlite3 is required to execute the documented V1 DDL.'
    } else {
        $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("lan-chat-docs-$([guid]::NewGuid())")
        New-Item -ItemType Directory -Path $tempDir | Out-Null
        try {
            $documentedSqlPath = Join-Path $tempDir 'documented-schema.sql'
            $documentedDbPath = Join-Path $tempDir 'documented-schema.db'
            $migratedDbPath = Join-Path $tempDir 'migrated-schema.db'
            [IO.File]::WriteAllText($documentedSqlPath, $documentedDdl, [Text.UTF8Encoding]::new($false))
            $documentedReadPath = $documentedSqlPath.Replace('\', '/')
            $sqliteOutput = & $sqlite.Source $documentedDbPath ".read '$documentedReadPath'" 2>&1
            if ($LASTEXITCODE -ne 0) {
                Add-Failure "Documented current DDL failed in SQLite: $($sqliteOutput -join ' ')"
            } else {
                $migrationSucceeded = $true
                foreach ($migrationName in @('migration_v1.sql', 'migration_v2.sql', 'migration_v3.sql', 'migration_v4.sql')) {
                    $sourceMigrationPath = Join-Path $repoRoot "core_rust/src/storage/$migrationName"
                    $tempMigrationPath = Join-Path $tempDir $migrationName
                    [IO.File]::WriteAllText(
                        $tempMigrationPath,
                        (Get-Content -LiteralPath $sourceMigrationPath -Raw),
                        [Text.UTF8Encoding]::new($false)
                    )
                    $migrationReadPath = $tempMigrationPath.Replace('\', '/')
                    $sqliteOutput = & $sqlite.Source $migratedDbPath ".read '$migrationReadPath'" 2>&1
                    if ($LASTEXITCODE -ne 0) {
                        Add-Failure "$migrationName failed in SQLite: $($sqliteOutput -join ' ')"
                        $migrationSucceeded = $false
                        break
                    }
                }
                if ($migrationSucceeded) {
                    $schemaQuery = "SELECT type || '|' || name || '|' || lower(replace(replace(replace(replace(sql, char(13), ''), char(10), ''), char(9), ''), ' ', '')) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name;"
                    $documentedSchema = @(& $sqlite.Source $documentedDbPath $schemaQuery)
                    $migratedSchema = @(& $sqlite.Source $migratedDbPath $schemaQuery)
                    if (@(Compare-Object $documentedSchema $migratedSchema).Count -ne 0) {
                        Add-Failure 'The documented current DDL differs from the schema produced by V1 through V4 migrations.'
                    }
                }
            }
        } finally {
            Remove-Item -LiteralPath $tempDir -Recurse -Force
        }
    }
}
$checks.Add('DOC-005 current SQLite schema')

# DOC-006: every requirement has interfaces, implementation tasks, and tests.
$testPlan = $contentByPath[(Resolve-Path (Join-Path $docRoot '08-test-plan.md')).Path]
$requirementIds = @(
    [regex]::Matches($testPlan, '(?m)^\| `(?<id>RQ-\d{3})` \|') |
        ForEach-Object { $_.Groups['id'].Value }
)
$matrixMatches = [regex]::Matches(
    $testPlan,
    '(?m)^\| (?<rq>RQ-\d{3}) \| (?<interfaces>[^|]+) \| (?<tasks>[^|]+) \| (?<tests>[^|]+) \|'
)
$matrixIds = @($matrixMatches | ForEach-Object { $_.Groups['rq'].Value })
if (@(Compare-Object $requirementIds $matrixIds).Count -ne 0) {
    Add-Failure 'Requirement definitions and traceability matrix rows differ.'
}
$guide = $contentByPath[(Resolve-Path (Join-Path $docRoot '07-implementation-guide.md')).Path]
$taskIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($match in [regex]::Matches($guide, '(?m)^### (?<id>[A-Z]+-\d{2})\b')) {
    [void]$taskIds.Add($match.Groups['id'].Value)
}
$testIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($match in [regex]::Matches($testPlan, '`(?<id>[A-Z0-9]+(?:-[A-Z0-9]+)*-\d{3})`')) {
    [void]$testIds.Add($match.Groups['id'].Value)
}
foreach ($row in $matrixMatches) {
    if ([string]::IsNullOrWhiteSpace($row.Groups['interfaces'].Value)) {
        Add-Failure "$($row.Groups['rq'].Value) has no interface mapping."
    }
    foreach ($id in Expand-IdReference $row.Groups['tasks'].Value) {
        if (-not $taskIds.Contains($id)) { Add-Failure "Unknown implementation task in trace matrix: $id" }
    }
    foreach ($id in Expand-IdReference $row.Groups['tests'].Value) {
        if (-not $testIds.Contains($id)) { Add-Failure "Unknown test in trace matrix: $id" }
    }
}
$checks.Add("DOC-006 requirement traceability ($($requirementIds.Count) requirements)")

# DOC-007: every deliverable has purpose, prerequisites, specification, example, exceptions, and checklist.
$requiredSections = @('目的', '前置知识', '示例', '异常', '检查表')
foreach ($file in $markdownFiles) {
    $content = $contentByPath[(Resolve-Path $file).Path]
    foreach ($section in $requiredSections) {
        if ($content -notmatch "(?m)^## .*?$([regex]::Escape($section)).*?$") {
            Add-Failure "$([IO.Path]::GetRelativePath($repoRoot, $file)) lacks a '$section' section."
        }
    }
    if ($content -notmatch '(?m)^#{1,2} .*?(?:规范|产品定义).*?$') {
        Add-Failure "$([IO.Path]::GetRelativePath($repoRoot, $file)) lacks a specification section or specification title."
    }
}
$checks.Add("DOC-007 required sections ($($markdownFiles.Count) documents)")

if ($failures.Count -gt 0) {
    Write-Error ("Documentation verification failed:`n - " + ($failures -join "`n - "))
    exit 1
}

foreach ($check in $checks) { Write-Host "PASS $check" }
Write-Host 'Documentation verification passed. DOC-008 remains a human review gate.'
