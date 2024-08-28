#!/usr/bin/env bash

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of tests.sh                                          │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├──┐VARIABLES
# │  ├── MAIN
# │  └── DERIVED
# ├──┐FUNCTIONS
# │  ├── PREPARATION
# │  ├──┐RUSTICLONE BACKUP
# │  │  ├── SEQUENTIAL
# │  │  └── PARALLEL
# │  ├──┐DISASTER SIMULATION
# │  │  ├── LOSING LOCAL FILES
# │  │  ├── LOSING CACHE
# │  │  ├── LOSING LOCAL BACKUP
# │  │  └── LOSING REMOTE BACKUP
# │  ├──┐RUSTICLONE RESTORE
# │  │  ├── SEQUENTIAL
# │  │  └── PARALLEL
# │  ├── RESULT
# │  └── MAIN
# ├── COMMANDS
# │
# └───────────────────────────────────────────────────────────────

set -euo pipefail

# ################################################################ VARIABLES
# ################################ MAIN

# must either be ~/.config/rustic or /etc/rustic
RUSTIC_PROFILES_DIR="$HOME/.config/rustic"

# can be any folder
RUSTICLONE_TEST_DIR="$HOME/rusticlone-tests"

# ################################ DERIVED

mapfile -d '' profile1Content << CONTENT
[repository]
repository = "$RUSTICLONE_TEST_DIR/local/Documents"
cache-dir = "$RUSTICLONE_TEST_DIR/cache"
password = "XXXXXX"

[[backup.sources]]
source = "$RUSTICLONE_TEST_DIR/source/docs"

[global.env]
RCLONE_CONFIG = "$RUSTIC_PROFILES_DIR/rclone-decrypted.conf"
CONTENT

mapfile -d '' profile2Content << CONTENT
[repository]
repository = "$RUSTICLONE_TEST_DIR/local/Pictures"
cache-dir = "$RUSTICLONE_TEST_DIR/cache"
password = "XXXXXX"

[[backup.sources]]
source = "$RUSTICLONE_TEST_DIR/source/pics"

[[backup.sources]]
source = "$RUSTICLONE_TEST_DIR/source/photos"

[global.env]
RCLONE_CONFIG = "$RUSTIC_PROFILES_DIR/rclone-encrypted.conf"
RCLONE_CONFIG_PASS = "YYYYYY"
CONTENT

mapfile -d '' profile3Content << CONTENT
[repository]
repository = "$RUSTICLONE_TEST_DIR/local/Passwords"
cache-dir = "$RUSTICLONE_TEST_DIR/cache"
password = "XXXXXX"

[[backup.sources]]
source = "$RUSTICLONE_TEST_DIR/source/passwords.kdbx"

[[backup.sources]]
source = [
  "$RUSTICLONE_TEST_DIR/source/secrets"
]

[global.env]
RCLONE_CONFIG = "$RUSTIC_PROFILES_DIR/rclone-encrypted.conf"
RCLONE_PASSWORD_COMMAND = "/usr/bin/python -c \"print('YYYYYY')\""
CONTENT

mapfile -d '' profileCommonContent << CONTENT
[global]
log-level = "debug"
log-file = "$RUSTICLONE_TEST_DIR/logs/rusticlone.log"
CONTENT

mapfile -d '' rcloneConfContentDecrypted << CONTENT
[gdrive]
type = local
CONTENT

mapfile -d '' rcloneConfContentEncrypted << CONTENT
# Encrypted rclone configuration File

RCLONE_ENCRYPT_V0:
LDDUg4mDyUxDwMtntnCaiUN+o9SexiohA8Y74ZYJmPD9KD8UjVtH9XYCL+3A6OGR7msabjvu0Gj2W8JRande
CONTENT

# ################################################################ FUNCTIONS
# ################################ PREPARATION

stopwatch_begin(){
 datebegin="$EPOCHSECONDS"
 export datebegin
}

stopwatch_end(){
 dateend="$EPOCHSECONDS"
 datediff="$((dateend-datebegin))"
 runtime="$(date -ud "@$datediff" -u +'%-Mm %-Ss')"
 echo "[KO] Tests completed succesfully in $runtime"
}

check_workdir(){
 if [[ "tests/tests.sh" != *$1 ]]; then
  echo "[KO] Please run this script from the main folder as 'tests/test.sh'"
  exit 1
 fi
}

logecho(){
 if [ "${2+isSet}" ]; then
  logFile="$2"
 else
  logFile="$RUSTICLONE_TEST_DIR/logs/rusticlone.log"
 fi
 echo "$1"
 echo " "  >> "$logFile"
 echo "$1" >> "$logFile"
}

print_space(){
 echo " "
 echo " "
}

print_warning(){
 echo "[!!] WARNING: this script will destroy the contents of \"$RUSTIC_PROFILES_DIR\" and \"$RUSTICLONE_TEST_DIR\""
 echo "[!!] Required programs: bash, coreutils, python-coverage, rustic, rclone"
 sleep 5
}

print_cleanup(){
 echo "[OK] Test completed, feel free to read test coverage and remove \"$RUSTIC_PROFILES_DIR\" and \"$RUSTICLONE_TEST_DIR\""
}

create_dirs(){
 echo "[OK] Creating directories"
 if [[ -d "$RUSTIC_PROFILES_DIR" ]]; then
  rm -r "$RUSTIC_PROFILES_DIR"
 fi
 if [[ -d "$RUSTICLONE_TEST_DIR" ]]; then
  rm -r "$RUSTICLONE_TEST_DIR"
 fi
 mkdir -p "$RUSTIC_PROFILES_DIR" "$RUSTICLONE_TEST_DIR"/{check,source/docs,source/pics,source/photos/deeply/nested,source/secrets,logs}
}

create_confs(){
 echo "[OK] Creating configurations"
 profile1Conf="$RUSTIC_PROFILES_DIR/Documents-test.toml"
 profile2Conf="$RUSTIC_PROFILES_DIR/Pictures-test.toml"
 profile3Conf="$RUSTIC_PROFILES_DIR/Passwords-test.toml"
 rcloneConfDecrypted="$RUSTIC_PROFILES_DIR/rclone-decrypted.conf"
 rcloneConfEncrypted="$RUSTIC_PROFILES_DIR/rclone-encrypted.conf"
 echo "${profile1Content[0]}" > "$profile1Conf"
 echo "${profile2Content[0]}" > "$profile2Conf"
 echo "${profile3Content[0]}" > "$profile3Conf"
 echo "${profileCommonContent[0]}" >> "$profile1Conf"
 echo "${profileCommonContent[0]}" >> "$profile2Conf"
 echo "${profileCommonContent[0]}" >> "$profile3Conf"
 echo "${rcloneConfContentDecrypted[0]}" > "$rcloneConfDecrypted"
 echo "${rcloneConfContentEncrypted[0]}" > "$rcloneConfEncrypted"
}

create_files(){
 # 10MB each
 echo "[OK] Creating files"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/docs/important.pdf"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/docs/veryimportant.pdf"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/docs/notsoimportant.docx"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/screenshot.png"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/opengraph.webp"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/funny.gif"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/photos/photo.jpeg"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/photos/deeply/nested/memory.avif"
 sameFile="$(head -c 10000000 /dev/urandom | base64)"
 echo "$sameFile" > "$RUSTICLONE_TEST_DIR/source/passwords.kdbx"
 echo "$sameFile" "almost" > "$RUSTICLONE_TEST_DIR/source/secrets/env"
 chmod 0600 "$RUSTICLONE_TEST_DIR/source/passwords.kdbx"
}

create_check_source(){
 echo "[OK] Creating checksums for source files"
 find "$RUSTICLONE_TEST_DIR/source" -type f -exec b2sum {} \; > "$RUSTICLONE_TEST_DIR/check/source.txt"
}

# ################################ RUSTICLONE BACKUP
# ################ SEQUENTIAL

rusticlone_backup(){
 logecho "[OK] Backing up with Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" backup
}

rusticlone_archive(){
 logecho "[OK] Archiving with Rusticlone"
 coverage run --append --module rusticlone.cli archive
}

rusticlone_upload(){
 logecho "[OK] Uploading with Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" upload
}

# ################ PARALLEL

rusticlone_backup_parallel(){
 echo "[OK] Backing up with Rusticlone using parallel mode"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" --parallel backup
}

rusticlone_archive_parallel(){
 echo "[OK] Archiving with Rusticlone using parallel mode"
 coverage run --append --module rusticlone.cli --parallel archive
}

rusticlone_upload_parallel(){
 echo "[OK] Uploading with Rusticlone using parallel mode"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" --parallel upload
}

# ################################ DISASTER SIMULATION
# ################ LOSING SOURCE FILES

destroy_source1(){
 echo "[OK] Destroying documents"
 rm -rf "$RUSTICLONE_TEST_DIR/source/docs"
}

destroy_source2(){
 echo "[OK] Destroying pictures"
 rm -rf "$RUSTICLONE_TEST_DIR/source/pics"
}

destroy_source3(){
 echo "[OK] Destroying passwords"
 rm -rf "$RUSTICLONE_TEST_DIR/source/passwords.kdbx"
}

destroy_source12(){
 echo "[OK] Destroying documents and pictures"
 rm -rf "$RUSTICLONE_TEST_DIR/source"
}

# ################ LOSING CACHE

destroy_cache(){
 echo "[OK] Destroying cache"
 rm -rf "$RUSTICLONE_TEST_DIR/cache"
}

# ################ LOSING LOCAL BACKUP

destroy_local1(){
 echo "[OK] Destroying local documents backup"
 rm -rf "$RUSTICLONE_TEST_DIR/local/Documents"
}

destroy_local2(){
 echo "[OK] Destroying local pictures backup"
 rm -rf "$RUSTICLONE_TEST_DIR/local/Pictures"
}

destroy_local12(){
 echo "[OK] Destroying local documents and pictures backup"
 rm -rf "$RUSTICLONE_TEST_DIR/local"
}

# ################ LOSING REMOTE BACKUP

destroy_remote1(){
 echo "[OK] Destroying remote documents backup"
 rm -rf "$RUSTICLONE_TEST_DIR/remote/Documents"
}

destroy_remote2(){
 echo "[OK] Destroying remote pictures backup"
 rm -rf "$RUSTICLONE_TEST_DIR/remote/Pictures"
}

destroy_remote12(){
 echo "[OK] Destroying remote documents and pictures backups"
 rm -rf "$RUSTICLONE_TEST_DIR/remote"
}

# ################################ RUSTICLONE RESTORE
# ################ SEQUENTIAL

rusticlone_restore(){
 logecho "[OK] Restoring from Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" restore
}

rusticlone_restore_flags(){
 logecho "[OK] Restoring from Rusticlone" "$RUSTICLONE_TEST_DIR/logs/log-specified-in-args.log"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" -P "Documents-test" --log-file "$RUSTICLONE_TEST_DIR/logs/log-specified-in-args.log" restore
 logecho "[OK] Restoring from Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" -P "Passwords-test" --ignore "common" restore
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" -P "Pictures-test" restore
}

rusticlone_download(){
 logecho "[OK] Downloading from Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" download
}

rusticlone_extract(){
 logecho "[OK] Extracting from Rusticlone"
 coverage run --append --module rusticlone.cli extract
}

# ################ PARALLEL

rusticlone_restore_parallel(){
 echo "[OK] Restoring with Rusticlone using parallel mode"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" --parallel restore
}

rusticlone_download_parallel(){
 echo "[OK] Downloading with Rusticlone using parallel mode"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" --parallel download
}

rusticlone_extract_parallel(){
 echo "[OK] Extracting with Rusticlone using parallel mode"
 coverage run --append --module rusticlone.cli --parallel extract
}

# ################################ RESULT

check_source(){
 echo "[OK] Checking sums for source files"
 b2sum --check "$RUSTICLONE_TEST_DIR/check/source.txt" || exit 1
}

create_coverage(){
 coverage html
 rm -rf "tests/coverage"
 mv "htmlcov" "$RUSTICLONE_TEST_DIR/coverage"
}

check_coverage(){
 echo "[OK] Read the coverage report by running:"
 echo " "
 echo "    firefox \"$RUSTICLONE_TEST_DIR/coverage/index.html\""
 echo " "
}

# ################################ MAIN

main(){
 # preparation
 check_workdir "$0"
 print_warning
 stopwatch_begin
 create_dirs
 create_confs
 create_files
 create_check_source
 
 # backup
 rusticlone_backup
 print_space
 destroy_local1
 destroy_remote2
 print_space
 rusticlone_backup_parallel
 print_space
 destroy_remote1
 destroy_local2
 print_space
 rusticlone_archive
 print_space
 destroy_cache
 print_space
 rusticlone_upload_parallel
 print_space
 destroy_remote2
 destroy_cache
 print_space
 rusticlone_archive_parallel
 rusticlone_upload
 print_space
 destroy_source12
 destroy_source3
 destroy_local2
 print_space

 # restore
 rusticlone_restore
 print_space
 destroy_source1
 destroy_local2
 print_space
 rusticlone_restore_parallel
 print_space
 check_source
 destroy_local1
 destroy_source2
 print_space
 rusticlone_download_parallel
 rusticlone_extract
 print_space
 check_source
 destroy_cache
 print_space
 rusticlone_download
 rusticlone_extract_parallel
 print_space
 check_source
 print_space
 destroy_cache
 destroy_source1
 destroy_local2
 print_space
 rusticlone_restore
 print_space
 destroy_source2
 destroy_local1
 print_space
 rusticlone_restore_flags
 print_space

 # result
 check_source
 rusticlone_backup
 print_space
 create_coverage
 check_coverage
 stopwatch_end
}

# ################################################################ COMMANDS

main "$@"
