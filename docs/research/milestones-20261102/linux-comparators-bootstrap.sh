#!/bin/sh
set -eu

test "$(uname -m)" = aarch64
test "$(id -u)" -ne 0
task_root="$PWD/warm-milestones-20261103/comparators"
mkdir -p "$task_root/downloads" "$task_root/evidence"
sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends libreadline-dev zlib1g-dev libicu-dev bison flex > "$task_root/evidence/build-dependencies.log" 2>&1
cd "$task_root/downloads"
curl -fsSL --connect-timeout 15 --max-time 300 --max-filesize 40000000 https://ftp.postgresql.org/pub/source/v18.6/postgresql-18.6.tar.bz2 -o postgresql-18.6.tar.bz2
printf '%s\n' '555610c24d53e4316da5b7d3fc25c279d96856d5e0e23ee308c328c5fa881d9f  postgresql-18.6.tar.bz2' | sha256sum -c -
tar -xjf postgresql-18.6.tar.bz2 -C "$task_root"
cd "$task_root/postgresql-18.6"
./configure --prefix="$task_root/postgres" --with-openssl > "$task_root/evidence/postgres-configure.log" 2>&1
make -j2 > "$task_root/evidence/postgres-build.log" 2>&1
make install > "$task_root/evidence/postgres-install.log" 2>&1
cd "$task_root"
git clone --depth 1 --branch v0.8.7 https://github.com/pgvector/pgvector.git pgvector > "$task_root/evidence/pgvector-fetch.log" 2>&1
cd pgvector
test "$(git rev-parse HEAD)" = f37c13f68b57d2c3472b2214fbcff699d6d34876
make -j2 PG_CONFIG="$task_root/postgres/bin/pg_config" > "$task_root/evidence/pgvector-build.log" 2>&1
make install PG_CONFIG="$task_root/postgres/bin/pg_config" > "$task_root/evidence/pgvector-install.log" 2>&1
cd "$task_root/downloads"
curl -fsSL --connect-timeout 15 --max-time 900 --max-filesize 2000000000 https://artifacts.opensearch.org/releases/bundle/opensearch/3.9.0/opensearch-3.9.0-linux-arm64.tar.gz -o opensearch-3.9.0-linux-arm64.tar.gz
printf '%s\n' '629b6a97104d6f925becd2372d902ea253a6209827435841c2cea9866c83567af776942b488110807370f9f504451bb4512f3bb1afb3c956386914c9474423fc  opensearch-3.9.0-linux-arm64.tar.gz' | sha512sum -c -
tar -xzf opensearch-3.9.0-linux-arm64.tar.gz -C "$task_root"
"$task_root/postgres/bin/postgres" --version > "$task_root/evidence/versions.txt"
cat "$task_root/postgres/share/extension/vector.control" >> "$task_root/evidence/versions.txt"
OPENSEARCH_JAVA_OPTS='-Xms256m -Xmx256m' "$task_root/opensearch-3.9.0/bin/opensearch" --version >> "$task_root/evidence/versions.txt" 2>&1
sha256sum "$task_root/postgres/bin/postgres" "$task_root/postgres/lib/vector.so" "$task_root/opensearch-3.9.0/bin/opensearch" > "$task_root/evidence/binaries.sha256"
printf '%s\n' complete > "$task_root/evidence/install-finished.txt"
