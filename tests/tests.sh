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
CONTENT

mapfile -d '' profile2Content << CONTENT
[repository]
repository = "$RUSTICLONE_TEST_DIR/local/Pictures"
cache-dir = "$RUSTICLONE_TEST_DIR/cache"
password = "XXXXXX"

[[backup.sources]]
source = "$RUSTICLONE_TEST_DIR/source/pics"
CONTENT

mapfile -d '' profileCommonContent << CONTENT
[global]
log-level = "debug"
log-file = "$RUSTICLONE_TEST_DIR/rusticlone.log"

[global.env]
RCLONE_CONFIG="$RUSTIC_PROFILES_DIR/rclone.conf"
CONTENT

mapfile -d '' rcloneConfContent << CONTENT
[gdrive]
type = local
CONTENT

# ################################################################ FUNCTIONS
# ################################ PREPARATION

check_workdir(){
 if [[ "tests/tests.sh" != *$1 ]]; then
  echo "[KO] Please run this script from the main folder as 'tests/test.sh'"
  exit 1
 fi
}

print_warning(){
 echo "[!!] WARNING: this script will destroy the contents of \"$RUSTIC_PROFILES_DIR\" and \"$RUSTICLONE_TEST_DIR\""
 echo "[!!] Required programs: coreutils, bash, python-coverage, rustic, rclone"
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
 mkdir -p "$RUSTIC_PROFILES_DIR" "$RUSTICLONE_TEST_DIR"/{source/docs,source/pics}
}

create_confs(){
 echo "[OK] Creating configurations"
 profile1Conf="$RUSTIC_PROFILES_DIR/Documents.toml"
 profile2Conf="$RUSTIC_PROFILES_DIR/Pictures.toml"
 rcloneConf="$RUSTIC_PROFILES_DIR/rclone.conf"
 echo "${profile1Content[0]}" > "$profile1Conf"
 echo "${profile2Content[0]}" > "$profile2Conf"
 echo "${profileCommonContent[0]}" >> "$profile1Conf"
 echo "${profileCommonContent[0]}" >> "$profile2Conf"
 echo "${rcloneConfContent[0]}" > "$rcloneConf"
}

create_files(){
 # 10MB each
 echo "[OK] Creating files"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/docs/important.pdf"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/docs/veryimportant.pdf"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/docs/notsoimportant.docx"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/screenshot.png"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/photo.jpeg"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/opengraph.webp"
 head -c 10000000 /dev/urandom > "$RUSTICLONE_TEST_DIR/source/pics/funny.gif"
}

create_sums(){
 echo "[OK] Creating checksums"
 b2sum "$RUSTICLONE_TEST_DIR/source"/**/*.* > "$RUSTICLONE_TEST_DIR/check.txt"
}

# ################################ RUSTICLONE BACKUP
# ################ SEQUENTIAL

rusticlone_backup(){
 echo "[OK] Backing up with Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" backup
}

rusticlone_archive(){
 echo "[OK] Archiving with Rusticlone"
 coverage run --append --module rusticlone.cli archive
}

rusticlone_upload(){
 echo "[OK] Uploading with Rusticlone"
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
# ################ LOSING LOCAL FILES

destroy_source1(){
 echo "[OK] Destroying documents"
 rm -rf "$RUSTICLONE_TEST_DIR/source/docs"
}

destroy_source2(){
 echo "[OK] Destroying pictures"
 rm -rf "$RUSTICLONE_TEST_DIR/source/pics"
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
 echo "[OK] Restoring from Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" restore
}

rusticlone_restore_flags(){
 echo "[OK] Restoring from Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" -P "Documents" --log-file "$RUSTICLONE_TEST_DIR/documents-restore.log" restore
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" -P "Pictures" --ignore "common" restore
}

rusticlone_download(){
 echo "[OK] Downloading from Rusticlone"
 coverage run --append --module rusticlone.cli --remote "gdrive:/$RUSTICLONE_TEST_DIR/remote" download
}

rusticlone_extract(){
 echo "[OK] Extracting from Rusticlone"
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

check_sums(){
 echo "[OK] Checking sums"
 b2sum --check "$RUSTICLONE_TEST_DIR/check.txt" || exit 1
}

create_coverage(){
 coverage html
 rm -rf "tests/coverage"
 mv "htmlcov" "$RUSTICLONE_TEST_DIR/coverage"
}

check_coverage(){
 echo "[OK] Read the coverage report by running:"
 echo "firefox \"$RUSTICLONE_TEST_DIR/coverage/index.html\""
}

# ################################ MAIN

main(){
 # preparation
 check_workdir "$0"
 print_warning
 create_dirs
 create_confs
 create_files
 create_sums
 # backup
 rusticlone_backup
 destroy_local1
 destroy_remote2
 rusticlone_backup_parallel
 destroy_remote1
 destroy_local2
 rusticlone_archive
 destroy_cache
 rusticlone_upload_parallel
 destroy_remote2
 destroy_cache
 rusticlone_archive_parallel
 rusticlone_upload
 destroy_source12
 destroy_local2
 # restore
 rusticlone_restore_parallel
 check_sums
 destroy_local1
 destroy_source2
 rusticlone_download_parallel
 rusticlone_extract
 check_sums
 destroy_cache
 rusticlone_download
 rusticlone_extract_parallel
 check_sums
 destroy_cache
 destroy_source1
 destroy_local2
 rusticlone_restore
 destroy_source2
 destroy_local1
 rusticlone_restore_flags
 # result
 check_sums
 create_coverage
 check_coverage
}

# ################################################################ COMMANDS

main "$@"
