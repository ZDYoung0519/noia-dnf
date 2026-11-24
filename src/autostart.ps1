# Embedded in the application. Values containing user or filesystem data are
# passed as process environment strings, never interpolated into script text.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')

function Test-ProtectedInstall($exe) {
    if ($exe.StartsWith('\\')) { return $false }
    $volume = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($exe))
    if ([IO.Path]::GetDirectoryName($exe) -eq $volume.RootDirectory.FullName) { return $false }
    if ($volume.DriveType -ne 'Fixed' -or $volume.DriveFormat -notin @('NTFS','ReFS') -or [IO.Path]::GetFileName($exe) -ne 'noia-dnf.exe') { return $false }
    $registration = Get-ItemProperty -LiteralPath 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Noia.DNF' -ErrorAction SilentlyContinue
    if (!$registration -or $registration.InstallLocation -ne [IO.Path]::GetDirectoryName($exe)) { return $false }

    # Only SYSTEM, Administrators and TrustedInstaller may replace the executable,
    # plant a DLL next to it, or change ownership/permissions along the path.
    $trusted = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    $sidType = [Security.Principal.SecurityIdentifier]
    $item = Get-Item -LiteralPath $exe
    while ($item) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
        $acl = Get-Acl -LiteralPath $item.FullName
        if ($acl.GetOwner($sidType).Value -notin $trusted) { return $false }
        $raw = [Security.AccessControl.RawSecurityDescriptor]::new($acl.GetSecurityDescriptorBinaryForm(), 0)
        if ($null -eq $raw.DiscretionaryAcl) { return $false }
        # At the volume root, creating siblings cannot replace the install directory.
        $mask = if ($item.FullName -eq [IO.Path]::GetPathRoot($item.FullName)) { 0xD0040 } else { 0xD0156 }
        foreach ($rule in $acl.GetAccessRules($true, $true, $sidType)) {
            if ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) { continue }
            if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -notin $trusted -and
                ([int]$rule.FileSystemRights -band ($mask -bor 0x50000000))) { return $false }
        }
        if ($item -is [IO.FileInfo]) { $item = $item.Directory } else { $item = $item.Parent }
    }
    return $true
}

try {
    $sid = ([Security.Principal.NTAccount]::new($env:NOIA_SESSION_ACCOUNT)).Translate([Security.Principal.SecurityIdentifier]).Value
    $exe = $env:NOIA_STARTUP_EXE
    $name = "Noia.DNF.Logon.$sid"
    $service = New-Object -ComObject Schedule.Service
    $service.Connect()
    $folder = $service.GetFolder('\')
    try { $task = $folder.GetTask($name) }
    catch { if ($_.Exception.GetBaseException().HResult -ne -2147024894) { throw }; $task = $null }
    $owns = $task -and $task.Definition.Actions.Count -eq 1 -and $task.Definition.Actions.Item(1).Path -eq $exe -and $task.Definition.Actions.Item(1).Arguments -eq '--autostart'
    switch ($env:NOIA_STARTUP_OP) {
        'status' { if ($owns -and $task.Enabled -and $task.Definition.Triggers.Item(1).Enabled) { 'enabled' } else { 'disabled' } }
        'disable' { if ($owns) { $folder.DeleteTask($name, 0) }; 'disabled' }
        'enable' {
            if ($exe.Contains('%')) { throw 'Task Scheduler expands percent variables in paths. Move NoiA to a path without %.' }
            $definition = $service.NewTask(0)
            $definition.RegistrationInfo.Description = 'Start NoiA DNF in the notification area at sign-in.'
            $definition.Principal.UserId = $sid
            $definition.Principal.LogonType = 3 # TASK_LOGON_INTERACTIVE_TOKEN
            $protected = $false
            if ($sid -eq [Security.Principal.WindowsIdentity]::GetCurrent().User.Value) {
                try { $protected = Test-ProtectedInstall $exe } catch { $protected = $false }
            }
            if ($protected) {
                $definition.Principal.RunLevel = 1 # TASK_RUNLEVEL_HIGHEST
            }
            $trigger = $definition.Triggers.Create(9) # TASK_TRIGGER_LOGON
            $trigger.UserId = $sid
            $action = $definition.Actions.Create(0)
            $action.Path = $exe
            $action.Arguments = '--autostart'
            $action.WorkingDirectory = [IO.Path]::GetDirectoryName($exe)
            $definition.Settings.DisallowStartIfOnBatteries = $false
            $definition.Settings.StopIfGoingOnBatteries = $false
            $definition.Settings.ExecutionTimeLimit = 'PT0S'
            $null = $folder.RegisterTaskDefinition($name, $definition, 6, $sid, $null, 3)
            'enabled'
        }
        default { throw 'Unknown startup operation' }
    }
} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
