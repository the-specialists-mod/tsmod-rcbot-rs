#!/usr/bin/env bash
# Builds and packages the release archives for the platforms starting with $3 (`linux`,
# `windows`), into out/: tsmod-rcbot-rs-<version>-<platform><tag>.{tar.gz,zip} (stripped) and the
# same with -debug (debug info). <tag> marks the build, e.g. `-glibc2.36` for Linux. Each archive
# holds ts/ (dlls/, tsmod/rcbot/) to copy over the server's ts/, and the licence files.
# Usage: .github/package.sh <version> <tag> <platform prefix>
set -euo pipefail
version=$1 tag=$2 prefix=$3
mkdir -p out
for variant in "" --debug-info; do
  rm -rf dist stage
  # dist also fails a Windows DLL that imports a MinGW runtime DLL.
  cargo xtask dist $variant --platform "$prefix"
  suffix=""
  [[ -n $variant ]] && suffix="-debug"
  for dir in dist/*/; do
    platform=$(basename "$dir")
    name="tsmod-rcbot-rs-$version-$platform$tag$suffix"
    stage="stage/$name"
    mkdir -p "$stage/ts"
    cp -r "$dir"dlls "$dir"tsmod "$stage/ts/"
    cp LICENSE GPL-EXCEPTION.txt README.md "$stage/"
    case $platform in
      windows-*) (cd "$stage" && zip -qr "$OLDPWD/out/$name.zip" .) ;;
      *) tar -C "$stage" -czf "out/$name.tar.gz" . ;;
    esac
  done
done
rm -rf stage
ls -l out
