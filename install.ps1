# Installs the newest Agora release for this user on Windows, checked before anything changes:
#
#   irm https://raw.githubusercontent.com/ersinkoc/Agora/main/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/ersinkoc/Agora/main/install.ps1))) -Version 0.3.43 -Service
#
# - Downloads the program for Windows from the release, with its `.sha256` and `.sig`, and checks both: the checksum, and the
#   signature against the release key below (`ssh-keygen -Y verify` from Windows' OpenSSH client, the same check `agora update` makes).
# - Puts it in %LOCALAPPDATA%\Programs\agora (or over the agora.exe already on PATH, when that one is in your profile), without
#   administrator rights, adds the folder to your user PATH when it is missing, and pins the release key in <AGORA_HOME>\release-key
#   so `agora update` can check later releases.
# - On a machine already set up, offers to start Agora at login (`agora service install`); on a new one, `agora` asks that itself.
#   Nothing running is stopped or restarted: a running agora.exe is renamed aside, and keeps running until it restarts.
#
# Written for Windows PowerShell 5.1 and PowerShell 7. Parameters: -Version, -Dir, -NoModifyPath, -Service, -NoService, -Unsigned.
# The same through the environment: AGORA_VERSION, AGORA_INSTALL_DIR.

param(
    [string]$Version = $env:AGORA_VERSION,
    [string]$Dir = $env:AGORA_INSTALL_DIR,
    [switch]$NoModifyPath,
    [switch]$Service,
    [switch]$NoService,
    [switch]$Unsigned
)

# The whole install is one function called on the last line, so a download cut short runs nothing.
function Install-AgoraRelease {
    param([string]$Version, [string]$Dir, [bool]$NoModifyPath, [bool]$Service, [bool]$NoService, [bool]$Unsigned)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest's progress bar makes 5.1 downloads many times slower.

    # The product's name and where it is published: the one place they appear in this script.
    $Bin = 'agora'
    $Repo = 'ersinkoc/Agora'
    # docs/security/release-key.pub (scripts/check-docs.sh keeps the two the same).
    $ReleaseKey = 'agora-release ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAuO/WmQDYmsECh5NeOs8iIwdY56M9/CcV94HGYmfiAK'

    $base = "https://github.com/$Repo/releases"
    $target = 'x86_64-pc-windows-msvc'
    function Fail([string]$why) { throw "error: $why" }

    if (-not [Environment]::Is64BitOperatingSystem) { Fail "no release for 32-bit Windows" }
    # Windows on Arm runs the x64 build under emulation.
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch {}

    if ($Version) { $Version = $Version.TrimStart('v') }
    else {
        # The newest release, from where `releases/latest` redirects (no API call, no rate limit); the API if that fails.
        $tag = ''
        try {
            $req = [Net.HttpWebRequest]::Create("$base/latest")
            $req.AllowAutoRedirect = $false
            $req.Method = 'HEAD'
            $req.UserAgent = "$Bin-install"
            try { $resp = $req.GetResponse() }
            catch {
                # PowerShell 7 (.NET) may report the redirect itself as an exception that carries the response.
                $e = $_.Exception
                while ($e -and -not ($e -is [Net.WebException])) { $e = $e.InnerException }
                if (-not $e -or -not $e.Response) { throw }
                $resp = $e.Response
            }
            $where = [string]$resp.Headers['Location']
            $resp.Close()
            $tag = $where.TrimEnd('/').Split('/')[-1]
        }
        catch {}
        if (-not $tag.StartsWith('v')) {
            try { $tag = [string](Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Repo/releases/latest").tag_name }
            catch { Fail "could not find the newest release at $base/latest" }
        }
        if (-not $tag.StartsWith('v')) { Fail "could not find the newest release at $base/latest" }
        $Version = $tag.Substring(1)
    }
    if ($Version -notmatch '^[0-9][0-9.]*$') { Fail "not a release version: $Version" }

    $name = "$Bin-$target.exe"
    $url = "$base/download/v$Version/$name"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("$Bin-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Host "Downloading $Bin $Version for $target"
        $file = Join-Path $tmp $name
        foreach ($pair in @(@($url, $file), @("$url.sha256", "$file.sha256"))) {
            try { Invoke-WebRequest -UseBasicParsing -Uri $pair[0] -OutFile $pair[1] }
            catch { Fail "download failed: $($pair[0]) ($($_.Exception.Message))" }
        }

        $signersProbe = Join-Path $tmp 'empty.txt'
        [IO.File]::WriteAllText($signersProbe, '')
        $want = ((Get-Content -Raw -LiteralPath "$file.sha256").Trim() -split '\s+')[0].ToLowerInvariant()
        $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant()
        if ($want.Length -ne 64 -or $want -ne $got) {
            Fail "the checksum does not match: the download is damaged or not the release (nothing was installed)"
        }

        $keygen = Get-Command ssh-keygen.exe -ErrorAction SilentlyContinue
        if ($keygen) {
            # OpenSSH before 8.1 (Windows 10 1809 to 20H2 ship 7.7) cannot check signatures: it says "unknown option -- Y".
            $probe = Join-Path $tmp 'probe.txt'
            $null = Start-Process -FilePath $keygen.Source -NoNewWindow -Wait -PassThru -ArgumentList @('-Y', 'verify') `
                -RedirectStandardInput $signersProbe -RedirectStandardOutput (Join-Path $tmp 'probe-out.txt') -RedirectStandardError $probe
            if ((Get-Content -Raw -LiteralPath $probe -ErrorAction SilentlyContinue) -match 'option -- Y') { $keygen = $null }
        }
        if ($keygen) {
            try { Invoke-WebRequest -UseBasicParsing -Uri "$url.sig" -OutFile "$file.sig" }
            catch { Fail "download failed: $url.sig ($($_.Exception.Message))" }
            $signers = Join-Path $tmp 'allowed_signers'
            [IO.File]::WriteAllText($signers, "$ReleaseKey`n", (New-Object Text.UTF8Encoding($false)))
            # The program goes to ssh-keygen's standard input as a file: PowerShell's pipeline would turn its bytes into text.
            $p = Start-Process -FilePath $keygen.Source -NoNewWindow -Wait -PassThru `
                -ArgumentList @('-Y', 'verify', '-f', "`"$signers`"", '-I', 'agora-release', '-n', 'file', '-s', "`"$file.sig`"") `
                -RedirectStandardInput $file -RedirectStandardOutput (Join-Path $tmp 'out.txt') -RedirectStandardError (Join-Path $tmp 'err.txt')
            if ($p.ExitCode -ne 0) {
                Fail "the release signature does not verify (nothing was installed); OpenSSH 8.1 or newer is needed for -Y verify"
            }
            Write-Host "Checksum and release signature verified"
        }
        elseif ($Unsigned) {
            Write-Warning "no ssh-keygen that checks signatures (OpenSSH 8.1 or newer): only the checksum was checked (-Unsigned)"
        }
        else {
            Fail ("ssh-keygen from OpenSSH 8.1 or newer is needed to check the release signature (it is missing or too old). Add " +
                "Settings > System > Optional features > OpenSSH Client and keep Windows updated, or install a newer " +
                "Win32-OpenSSH release (github.com/PowerShell/Win32-OpenSSH); or pass -Unsigned to rely on the checksum alone")
        }

        # Where it goes: -Dir; else over the copy already on PATH when it is in this user's profile (a login task runs that path);
        # else %LOCALAPPDATA%\Programs\agora.
        $previous = (Get-Command "$Bin.exe" -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
        if (-not $Dir) {
            if ($previous -and $previous.StartsWith($env:USERPROFILE, [StringComparison]::OrdinalIgnoreCase)) {
                $Dir = Split-Path -Parent $previous
            }
            else {
                $Dir = Join-Path $env:LOCALAPPDATA "Programs\$Bin"
            }
        }
        New-Item -ItemType Directory -Force -Path $Dir | Out-Null
        $dest = Join-Path $Dir "$Bin.exe"
        $oldVersion = ''
        if (Test-Path -LiteralPath $dest) {
            try { $oldVersion = ((& $dest --version 2>$null) -split '\s+')[-1] } catch {}
        }
        # Copies renamed aside by earlier installs, gone once nothing runs them.
        Get-ChildItem -LiteralPath $Dir -Filter "$Bin.exe.old-*" -ErrorAction SilentlyContinue |
            ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }
        # A running agora.exe cannot be overwritten, but it can be renamed: it keeps running from its new name until it restarts.
        if (Test-Path -LiteralPath $dest) {
            Move-Item -LiteralPath $dest -Destination "$dest.old-$([DateTime]::UtcNow.ToString('yyyyMMddHHmmss'))" -Force
        }
        Move-Item -LiteralPath $file -Destination $dest -Force
        $shown = (& $dest --version 2>$null)
        if (-not $shown) { $shown = $Version }
        Write-Host "Installed $dest ($shown)"
        if ($previous -and -not [string]::Equals($previous, $dest, [StringComparison]::OrdinalIgnoreCase)) {
            Write-Host "note: another $Bin is at $previous; whichever folder comes first in PATH wins"
        }

        # The user PATH, read and written without expanding it (a %VARIABLE% in it stays one).
        $pathNote = ''
        $envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        $userPath = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $parts = @($userPath -split ';' | Where-Object { $_ })
        $onPath = $parts | Where-Object { [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ieq $Dir.TrimEnd('\') }
        if (-not $onPath) {
            if (-not $NoModifyPath) {
                $envKey.SetValue('Path', (($parts + $Dir) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
                # Tell open programs (Explorer, new terminals) that the environment changed.
                try {
                    if (-not ('AgoraInstall.Env' -as [type])) {
                        Add-Type -Namespace AgoraInstall -Name Env -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@
                    }
                    $r = [UIntPtr]::Zero
                    [void][AgoraInstall.Env]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$r)
                }
                catch {}
                Write-Host "Added $Dir to your user PATH"
                $pathNote = "Open a new terminal to use $Bin."
            }
            else {
                $pathNote = "$Dir is not on PATH: add it, or run $dest."
            }
        }
        $envKey.Close()
        if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $Dir.TrimEnd('\') })) { $env:Path = "$Dir;$env:Path" }

        # The release key for `agora update`, unless one is pinned already.
        # As agora finds it: AGORA_HOME, else HOME (Git for Windows and MSYS set it), else USERPROFILE.
        $home_ = if ($env:AGORA_HOME) { $env:AGORA_HOME } elseif ($env:HOME) { Join-Path $env:HOME ".$Bin" } else { Join-Path $env:USERPROFILE ".$Bin" }
        $keyFile = Join-Path $home_ 'release-key'
        if (Test-Path -LiteralPath $keyFile) {
            if ((Get-Content -Raw -LiteralPath $keyFile).Trim() -ne $ReleaseKey) {
                Write-Host "note: $keyFile holds another key than this installer's; it was left as it is"
            }
        }
        else {
            New-Item -ItemType Directory -Force -Path $home_ | Out-Null
            [IO.File]::WriteAllText($keyFile, "$ReleaseKey`n", (New-Object Text.UTF8Encoding($false)))
        }

        if (Test-Path -LiteralPath (Join-Path $home_ 'machine.toml')) {
            $doService = $Service
            if (-not $Service -and -not $NoService -and [Environment]::UserInteractive) {
                try {
                    $answer = Read-Host "Start $Bin when you log in? [Y/n]"
                    $doService = ($answer -eq '' -or $answer -match '^(y|yes)$')
                }
                catch { $doService = $false }   # -NonInteractive: nobody to ask.
            }
            if ($doService) {
                # Idempotent: a login task set up already is left as it is (and never restarted).
                & $dest service install
                if ($LASTEXITCODE -ne 0) { Write-Warning "'$Bin service install' did not finish; run it again to see why" }
            }
            if ($oldVersion -and $oldVersion -ne $Version) {
                Write-Host "$Bin keeps running $oldVersion until it restarts ($Bin down, then $Bin). Your agents keep running either way."
            }
        }
        else {
            Write-Host "Next: run '$Bin' to set this machine up (it asks whether to start $Bin when you log in)."
        }
        if ($pathNote) { Write-Host $pathNote }
    }
    finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-AgoraRelease -Version $Version -Dir $Dir -NoModifyPath:$NoModifyPath -Service:$Service -NoService:$NoService -Unsigned:$Unsigned
