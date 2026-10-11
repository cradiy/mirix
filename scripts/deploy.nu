#!/usr/bin/env nu

# Run as the desktop user. Only installation into a protected bin directory uses sudo.
def checked [program: string, ...args: string] {
    run-external $program ...$args
    if $env.LAST_EXIT_CODE != 0 {
        error make {msg: $"Command failed: ($program) ($args | str join ' ')"}
    }
}

def captured [program: string, ...args: string] {
    let result = (run-external $program ...$args | complete)
    if $result.exit_code != 0 {
        error make {msg: $"Command failed: ($program)\n($result.stderr)"}
    }
    $result.stdout | str trim
}

def install-command [elevated: bool, program: string, ...args: string] {
    if $elevated {
        checked sudo $program ...$args
    } else {
        checked $program ...$args
    }
}

def install-binary [source: string, destination: string, elevated: bool] {
    let directory = ($destination | path dirname)
    install-command $elevated mkdir '-p' '--' $directory
    let template = ($directory | path join '.mirix-install.XXXXXX')
    let temporary = if $elevated {
        captured sudo mktemp $template
    } else {
        captured mktemp $template
    }
    try {
        install-command $elevated install '-m755' '--' $source $temporary
        install-command $elevated mv '-fT' '--' $temporary $destination
    } catch {|failure|
        # Remove only the unique temporary file created by this invocation.
        install-command $elevated rm '-f' '--' $temporary
        error make {msg: $failure.msg}
    }
    print $"Installed ($destination)"
}

def bundled-skins [repo: path] {
    ls ($repo | path join skins)
    | where type == dir
    | get name
    | where {|directory| ($directory | path join skin.toml | path type) == file }
    | sort
}

def desktop-executable [path: string] {
    # Desktop Exec quoting has two escape layers; it is not shell quoting.
    if ($path | str contains '=') or ($path | str contains (char newline)) or ($path | str contains (char carriage_return)) {
        error make {msg: 'Desktop executable paths cannot contain = or line breaks.'}
    }
    let escaped = ($path
        | str replace --all '\' '\\\\'
        | str replace --all '"' '\\"'
        | str replace --all '`' '\\`'
        | str replace --all '$' '\\$'
        | str replace --all '%' '%%')
    '"' + $escaped + '"'
}

def install-desktop [repo: path, data_home: path, panel_command: string] {
    let applications = ($data_home | path join applications)
    let icons = ($data_home | path join icons hicolor scalable apps)
    mkdir $applications $icons
    open --raw ($repo | path join deploy mirix.desktop)
        | str replace 'Exec=mirix-panel --settings' $"Exec=($panel_command) --settings"
        | save --force ($applications | path join mirix.desktop)
    checked install '-m644' '--' ($repo | path join deploy mirix.svg) ($icons | path join mirix.svg)
    if (which update-desktop-database | is-not-empty) {
        checked update-desktop-database $applications
    }
    print $"Installed desktop entry ($applications | path join mirix.desktop)"
}

def install-skins [sources: list<string>, destination: path, update: bool] {
    mkdir $destination
    for source in $sources {
        let skin = ($source | path basename)
        let target = ($destination | path join $skin)
        if not ($target | path exists) {
            cp -r $source $target
            print $"Installed skin ($skin)"
        } else if $update {
            if ((^test -L $target | complete).exit_code == 0) {
                error make {msg: $"Refusing to replace symlink: ($target)"}
            }
            # Backups are outside discovery roots and never appear as additional skins.
            let backups = ($destination | path dirname | path join skin-backups)
            mkdir $backups
            let staging = (captured mktemp '-d' ($backups | path join $"($skin).XXXXXX"))
            let prepared = ($staging | path join replacement)
            let saved = ($staging | path join original)
            cp -r $source $prepared
            mv $target $saved
            try {
                mv $prepared $target
            } catch {|failure|
                mv $saved $target
                error make {msg: $failure.msg}
            }
            print $"Updated skin ($skin); previous files preserved in ($saved)"
        }
    }
}

# Build and deploy Mirix for the current desktop user without modifying personal data.
def main [
    --bin-dir: path = '/usr/local/bin' # Must match the existing service's ExecStart.
    --no-restart                    # Install without contacting systemd or restarting Mirix.
    --dry-run                       # Show the deployment paths without building or writing files.
    --update-skins                  # Replace bundled skins and preserve complete installed copies.
] {
    if (captured id '-u') == '0' {
        error make {msg: 'Run this script as your desktop user, not with sudo. It requests sudo only when needed.'}
    }
    let repo = ($env.FILE_PWD | path dirname)
    let destination = ($bin_dir | path expand)
    let panel_command = (desktop-executable ($destination | path join mirix-panel))
    let data_home = ($env.XDG_DATA_HOME? | default ($env.HOME | path join '.local/share'))
    let data_root = ($data_home | path join mirix | path expand)
    let methods = ($data_root | path join input-methods)
    let skins = ($data_root | path join skins)
    let skin_sources = (bundled-skins $repo)
    let engines = [keyboard pinyin]

    print $"Binaries:     ($destination)"
    print $"Input methods: ($methods)/<name>/engine"
    print $"Skins:         ($skins)"
    print $"Desktop entry: ($data_home | path join applications mirix.desktop)"
    print $"App icon:      ($data_home | path join icons hicolor scalable apps mirix.svg)"
    print (if $update_skins { 'Skin policy: update bundled skins; preserve installed copies in mirix/skin-backups.' } else { 'Skin policy: copy missing skins only.' })
    print $"Bundled skins: ($skin_sources | each {|source| $source | path basename } | str join ', ')"
    print 'Configuration, dictionaries, profiles and learning data are left untouched.'
    print $"Engines: ($engines | str join ', ')"
    if $dry_run {
        print $"Restart existing user service: (not $no_restart)"
        return
    }

    # Validate the service before spending time on a build or changing installed files.
    let restart = if $no_restart {
        false
    } else {
        let state = (captured systemctl '--user' show mirix.service '--property=LoadState' '--value')
        if $state == 'not-found' {
            print 'No mirix.service installed; deploy only. See README.md for service setup.'
            false
        } else if $state != 'loaded' {
            error make {msg: $"mirix.service is not usable: ($state)"}
        } else {
            let command = (captured systemctl '--user' show mirix.service '--property=ExecStart' '--value')
            let expected = ($destination | path join mirixd)
            if not ($command | str contains $"path=($expected) ;") {
                error make {msg: $"mirix.service does not start ($expected). Use --bin-dir matching ExecStart, or --no-restart.\n($command)"}
            }
            true
        }
    }

    cd $repo
    let metadata = (captured cargo metadata '--no-deps' '--format-version' '1' | from json)
    # Explicitly build for this machine even if Cargo has a cross-compilation default.
    let host = (captured rustc '-vV' | lines | where {|line| $line starts-with 'host: '} | first | str replace 'host: ' '')
    checked cargo build '--release' '--target' $host '-p' mirixd '-p' mirix-panel '-p' mirix-engine-keyboard '-p' mirix-engine-pinyin '--bins'
    let artifacts = ($metadata.target_directory | path join $host release)
    for binary in ([mirixd mirix-panel] ++ ($engines | each {|method| $"mirix-engine-($method)" })) {
        if not (($artifacts | path join $binary) | path exists) {
            error make {msg: $"Missing build artifact: ($binary)"}
        }
    }

    mut parent = $destination
    while not ($parent | path exists) {
        $parent = ($parent | path dirname)
    }
    let elevated = ((^test -w $parent | complete).exit_code != 0)
    if $elevated { checked sudo '-v' }

    for binary in [mirixd mirix-panel] {
        install-binary ($artifacts | path join $binary) ($destination | path join $binary) $elevated
    }
    for method in $engines {
        install-binary ($artifacts | path join $"mirix-engine-($method)") ($methods | path join $method engine) false
    }
    install-skins $skin_sources $skins $update_skins
    install-desktop $repo $data_home $panel_command

    if $restart {
        checked systemctl '--user' restart mirix.service
        checked systemctl '--user' is-active mirix.service
        print 'Deployment complete; mirix.service restarted. Check the tray and try typing.'
    } else {
        print 'Deployment complete. Restart your running Mirix instance to use the new binaries.'
    }
}
