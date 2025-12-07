# aptmirs

**aptmirs** is a simple tool to mirror apt/deb repositories. 

## Features

* All downloads are verified with their pre-recorded checksum while downloading.
* The tool can filter out which arch-builds you want to mirror, as well as selectively filter out
  components.
* Updating an existing mirror is done by comparing the new Release file with the existing one, and
  only if there are any changes will the operation continue. Similarly, any file that already
  exists with the same checksum will not be downloaded again. This should make keeping an up
  to date mirror of a repository less stressful for the source servers, which _probably_ would
  make it more okay with more frequent updates than the recommended schedule of every 6 hours.
* During an update, metadata files (Packages, Release, etc) are kept in a temporary folder until
  all packages are downloaded. When completed, the files are moved into the target mirror folder.
  This should limit the time the repository is unavailable during an update.

## Configuration

aptmirs uses a config file format that is very similar to the sources.list file format. The
default configuration file location is `/etc/apt/mirror.list`. Here's a basic example:

```
deb http://ftp.se.debian.org/debian             trixie            main contrib non-free non-free-firmware
deb http://ftp.se.debian.org/debian             trixie-updates    main contrib non-free non-free-firmware
deb http://ftp.se.debian.org/debian             trixie-backports  main contrib non-free non-free-firmware
deb http://security.debian.org/debian-security  trixie-security   main contrib non-free non-free-firmware
```

### Configuration options

These options can be set as part of the configuration file:

| Config option | Description |
| ------------- | ----------- |
| arch          | The architecture to download packages for, e.g: `amd64`, `arm64`, etc. The default value is `amd64`. |
| di_arch       | The debian installer image architecture to download, e.g. `amd64`, `arm64`, etc. |
| pgp_pub_key   | Specify a PGP signing key to verify the repository. Any other key provided via the `--pgp-key-path` option will not be used. `pgp_verify` will be set to true if this option is set. |
| pgp_verify    | Whether or not to verify the PGP signature of the release file. If no signature is available, requiring verification will make the mirroring operation fail. This will also require you to provide a source of keys, usually via the `--pgp-key-path` option. The only recognized value is `true`. |
| udeb          | Whether or not to download udeb packages. The arch used for this is the same as for normal packages. The only recognized value is `true` |

### Configuration examples

Mirror *amd64* packages from a debian repository:

```
deb [arch=amd64] http://ftp.se.debian.org/debian  trixie  main contrib non-free non-free-firmware
```

Mirror *arm64* and *amd64* packages, and also download the *amd64* debian installer image from a debian repository:

```
deb [arch=arm64 arch=amd64 di_arch=amd64] http://ftp.se.debian.org/debian  trixie  main contrib non-free non-free-firmware
```

Mirror *amd64* packages and verify the PGP signature with a key in the specified directory.
Requires that the `--pgp-key-path` command option be set when syncing the mirror:

```
deb [arch=amd64 pgp_verify=true] http://ftp.se.debian.org/debian  trixie  main contrib non-free non-free-firmware
```

Mirror *amd64* packages from a debian repository and verify the PGP signature with a specified key:

```
deb [arch=amd64 pgp_pub_key=/etc/apt/trusted.gpg.d/debian-archive-trixie-stable.asc] http://ftp.se.debian.org/debian  trixie  main contrib non-free non-free-firmware
```

## Commands

aptmirs operations are run via the command-line, and can be supplemented with command line
options. There are three operations: `mirror`, `prune` and `verify`. 

* `mirror`: The default operation. Will be run if no command is specified.
* `prune`: Removes all unreferenced files inside a mirror, using the configuration to determine
  what files are valid. This is useful to remove old packages and free up space. Prune can also be
  run with `--dry-run`, which will only print the files that are unreferenced instead of deleting
  them.
* `verify`: Verifies a downloaded mirror using the configuration as source of truth, and makes
  sure that all files match their referenced checksum.

### Command options

| Long option    | Short option | ENV variable  | Description |
| ---------------| ------------ | ------------- | ----------- |
| --config       | -c           | CONFIG=       | The path to the config file containing the mirror options. [default: /etc/apt/mirror.list] |
| --force        | -f           | FORCE=        | Ignore the current release and package files and assume all metadata is stale. |
| --dl-threads   | -d           | DL_THREADS=   | The maximum number of concurrent mirror download tasks. *Works only with the `mirror` and `verify` commands*. [default: 8] |
| --dry-run      | -d           |               | Prints the files that the prune operation would delete. *Works only with the `prune` command*. |
| --mtime        | -m           |               | Set the mtime of all downloaded files to the Date field in the Release. *Works only with the `mirror` command*. |
| --output       | -o           | OUTPUT=       | The directory into where the mirrors will be downloaded. |
| --pgp-key-path | -p           | PGP_KEY_PATH= | Path to folder where PGP public keys reside. All valid keys will be used in signature verification where applicable. |
| --help         | -h           |               | Print help. |
| --version      | -V           |               | Print version. |

### Command examples

Mirror operation
```
./aptmirs --config ./mirror.list --output /opt/mirror-root
```

Prune operation
```
./aptmirs --config ./mirror.list --output /opt/mirror-root prune
```

Verify operation
```
./aptmirs --config ./mirror.list --output /opt/mirror-root verify
```