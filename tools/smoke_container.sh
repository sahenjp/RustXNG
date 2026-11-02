#!/usr/bin/env bash
set -euo pipefail
image=${1:?image required}
version=${2:?expected version required}
container="rustxng-smoke-$RANDOM-$RANDOM"
trap 'docker rm --force "$container" >/dev/null 2>&1 || true' EXIT
actual=$(docker run --rm "$image" --version)
[[ "$actual" == *" $version" ]] || { echo "unexpected container version: $actual" >&2; exit 1; }
docker run --detach --name "$container" --publish 127.0.0.1::29483 "$image" >/dev/null
port=$(docker inspect --format '{{(index (index .NetworkSettings.Ports "29483/tcp") 0).HostPort}}' "$container")
for _ in $(seq 1 40); do
  if curl --fail --silent "http://127.0.0.1:$port/healthz" >/dev/null; then
    curl --fail --silent "http://127.0.0.1:$port/config" > "${RUNNER_TEMP:-/tmp}/rustxng-container-$container.json"
    python3 -c 'import json,sys; value=json.load(open(sys.argv[1], encoding="utf-8")); assert value["version"].endswith(" " + sys.argv[2])' "${RUNNER_TEMP:-/tmp}/rustxng-container-$container.json" "$version"
    exit 0
  fi
  sleep 1
done
docker logs "$container" >&2
exit 1
