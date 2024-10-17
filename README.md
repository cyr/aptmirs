# aptmirs

**aptmirs** is a simple tool to mirror apt/deb repositories. 

All downloads are verified with their pre-recorded checksum while downloading.

The tool can filter out which arch-builds you want to mirror, as well as selectively filtering out components.

Updating an existing mirror is done by comparing the new Release file with the existing one, and only if there is any change will the operation continue. Similarly, any file that already exists with the same checksum will not be downloaded again.
This should make keeping an up to date mirror of a repository less stressful for the source servers, which _probably_ would make it more okay with more frequent updates than the recommended schedule of every 6 hours.

During an update metadata files (Packages, Release, etc) are kept in a temporary folder until all deb-packages are downloaded. When completed, the files are moved into the target mirror folder. This should limit the time the repository is dysfunctional during an update.

## Configuration

aptmirs uses a config file very similar to the sources.list format.

```
deb http://ftp.se.debian.org/debian             bookworm            main contrib non-free non-free-firmware
deb http://ftp.se.debian.org/debian             bookworm-updates    main contrib non-free non-free-firmware
deb http://ftp.se.debian.org/debian             bookworm-backports  main contrib non-free non-free-firmware
deb http://security.debian.org/debian-security  bookworm-security   main contrib non-free non-free-firmware
```

Architecture can be specified with [arch=value]. The default value is *amd64*.

```
deb [arch=amd64] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

To download debian installer image data, specify *di_arch* with the appropriate architecture.

```
deb [di_arch=amd64] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

Multiple options can be added inside the same bracket.

```
deb [arch=amd64 arch=arm64 di_arch=amd64] http://ftp.se.debian.org/debian  bookworm  main contrib non-free non-free-firmware
```

## Usage

```
./aptmirs --config ./mirror.list --output /opt/mirror-root
```
