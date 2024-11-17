# aptmirs

**aptmirs** is a simple tool to mirror apt/deb repositories. 

All downloads are verified with their pre-recorded checksum while downloading.

The tool can filter out which arch-builds you want to mirror, as well as selectively filtering out components.

Updating an existing mirror is done by comparing the new Release file with the existing one, and only if there is any change will the operation continue. Similarly, any file that already exists with the same checksum will not be downloaded again.
This should make keeping an up to date mirror of a repository less stressful for the source servers, which _probably_ would make it more okay with more frequent updates than the recommended schedule of every 6 hours.

During an update metadata files (Packages, Release, etc) are kept in a temporary folder until all deb-packages are downloaded. When completed, the files are moved into the target mirror folder. This should limit the time the repository is dysfunctional during an update.

## Commands

There are three operations: mirror, prune and verify.

* **mirror** is the default operation and will be run if no command is specified.
* **prune** will remove all unreferenced files inside a mirror, using the configuration to determine what files are valid. This is useful to remove old packages to free up space. Prune can also be run with --dry-run, which will only print the files that are unreferenced instead of deleting them.
* **verify** will verify a downloaded mirror, using the configuration as source of truth, and make sure that all files matches its referenced checksum.

## Configuration

aptmirs uses a config file very similar to the sources.list format.

```
deb http://ftp.se.debian.org/debian             bookworm            main contrib non-free non-free-firmware
deb http://ftp.se.debian.org/debian             bookworm-updates    main contrib non-free non-free-firmware
deb http://ftp.se.debian.org/debian             bookworm-backports  main contrib non-free non-free-firmware
deb http://security.debian.org/debian-security  bookworm-security   main contrib non-free non-free-firmware
```

### Config options

|Key|Value|
|---|------|
| arch        | The architecture to download packages for, e.g: `amd64`, `arm64`, etc. The default value is `amd64`. |
| di_arch     | The debian installer image architecture to download, e.g. `amd64`, `arm64`, etc. |
| udeb        | Whether or not to download udeb packages. The arch used for this is the same as for normal packages. The only recognized value is `true` |
| pgp_verify  | Whether or not to verify the PGP signature of the release file. If no signature is available, requiring verification will make the mirroring operation fail. This will also require you to provide a source of keys, usually via the `--pgp-key-path` option. The only recognized value is `true`. |
| pgp_pub_key | Specify a PGP signing key to verify the repository. Any other key provided via the `--pgp-key-path` option will not be used. `pgp_verify` will be set to true if this option is set. |


### Examples

Mirror debian repository for *amd64* packages.

```
deb [arch=amd64] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

Mirror debian repository for *arm64* and *amd64* packages, and also download debian installer image for *amd64*.

```
deb [arch=arm64 arch=amd64 di_arch=amd64] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

Mirror debian repository for *amd64* and verify the PGP signature.

```
deb [arch=amd64 pgp_verify=true] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

```
./aptmirs --config ./mirror.list --output /opt/mirror-root --pgp-key-path /etc/apt/trusted.gpg.d/
```

Mirror debian repository for *amd64* and verify the PGP signature with a specified key.

```
deb [arch=amd64 pgp_pub_key=/etc/apt/trusted.gpg.d/debian-archive-bookworm-stable.asc] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

## Usage

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