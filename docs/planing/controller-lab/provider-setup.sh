#!/bin/bash
# Run as root only inside controller-lab/ops after bootstrap apply.
set -euo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
test "$(uname -m)" = aarch64
test -f /run/vzctl-lab-provider-stage/admin.json
install -d -m 755 /opt/vzctl-lab
install -m 644 /run/vzctl-lab-provider-stage/requirements.txt /opt/vzctl-lab/requirements.txt
install -d -m 700 /var/lib/vzctl-lab /etc/vzctl-lab
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y postgresql postgresql-client python3 python3-venv curl ca-certificates build-essential
curl --fail --location --proto '=https' --tlsv1.2 --max-time 300 \
  https://go.dev/dl/go1.24.2.linux-arm64.tar.gz -o /var/lib/vzctl-lab/go.tar.gz
echo '756274ea4b68fa5535eb9fe2559889287d725a8da63c6aae4d5f23778c229f4b  /var/lib/vzctl-lab/go.tar.gz' | sha256sum -c -
tar -xzf /var/lib/vzctl-lab/go.tar.gz -C /var/lib/vzctl-lab
export PATH=/var/lib/vzctl-lab/go/bin:$PATH GOTOOLCHAIN=local GOMAXPROCS=2
# System services have no login HOME. Keep build/module caches inside this lab.
export GOCACHE=/var/lib/vzctl-lab/go-cache GOPATH=/var/lib/vzctl-lab/go-path
export GOMODCACHE=/var/lib/vzctl-lab/go-path/pkg/mod
install -d -m 700 "$GOCACHE" "$GOPATH" "$GOMODCACHE"
for item in minio:0d7408fc9969caf07de6a8c3a84f9fbb10a6739e mc:b00526b153a31b36767991a4f5ce2cced435ee8e; do
  name=${item%%:*}
  commit=${item#*:}
  install -d "/var/lib/vzctl-lab/$name-source"
  curl --fail --location --proto '=https' --tlsv1.2 --max-time 300 \
    "https://codeload.github.com/minio/$name/tar.gz/$commit" -o "/var/lib/vzctl-lab/$name.tar.gz"
  tar -xzf "/var/lib/vzctl-lab/$name.tar.gz" --strip-components=1 -C "/var/lib/vzctl-lab/$name-source"
  (cd "/var/lib/vzctl-lab/$name-source"; go build -mod=readonly -p 2 -o "/usr/local/bin/$name" .)
  chmod 755 "/usr/local/bin/$name"
  sudo -n -u vzctl-agent -- "/usr/local/bin/$name" --version >/dev/null
done
python3 - <<'PY'
import json, os, pathlib, subprocess
b=json.load(open("/run/vzctl-lab-provider-stage/admin.json"))
for name,value in b.items():
    assert len(value)==(20 if name=="access" else 64) and all(c in "0123456789abcdef" for c in value)
subprocess.run(["sudo","-u","postgres","psql","-X","-v","ON_ERROR_STOP=1"],
               input=("ALTER USER postgres PASSWORD '"+b["postgres"]+"';\n").encode(),
               stdout=subprocess.DEVNULL,check=True)
for config in pathlib.Path("/etc/postgresql").glob("*/main/postgresql.conf"):
    with config.open("a") as f:
        f.write("\nlisten_addresses = '127.0.0.1,10.123.0.10'\npassword_encryption = 'scram-sha-256'\n")
    with config.with_name("pg_hba.conf").open("a") as f:
        f.write("\nhost all all 10.123.0.0/24 scram-sha-256\nhost all all 10.124.0.0/24 scram-sha-256\nhost all all 10.244.0.0/16 scram-sha-256\n")
path=pathlib.Path("/etc/vzctl-lab/minio.env")
path.write_text("MINIO_ROOT_USER="+b["access"]+"\nMINIO_ROOT_PASSWORD="+b["secret"]+"\n")
os.chmod(path,0o600)
PY
cat >/etc/systemd/system/vzctl-lab-minio.service <<'EOF'
[Unit]
After=network-online.target
Wants=network-online.target
[Service]
EnvironmentFile=/etc/vzctl-lab/minio.env
ExecStart=/usr/local/bin/minio server /var/lib/vzctl-lab/objects --address 10.123.0.10:9000 --console-address 127.0.0.1:9001
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl restart postgresql
systemctl enable --now vzctl-lab-minio.service
python3 -m venv /opt/vzctl-lab/venv
/opt/vzctl-lab/venv/bin/pip install --only-binary=:all: --require-hashes -r /opt/vzctl-lab/requirements.txt
# The denied-access probe targets an existing, separately owned bucket.
/opt/vzctl-lab/venv/bin/python - <<'PY'
import json,time
import boto3
b=json.load(open("/run/vzctl-lab-provider-stage/admin.json"))
c=boto3.client("s3",endpoint_url="http://10.123.0.10:9000",region_name="us-east-1",
               aws_access_key_id=b["access"],aws_secret_access_key=b["secret"])
for attempt in range(30):
    try:
        buckets={x["Name"] for x in c.list_buckets()["Buckets"]}
        if "controller-lab-forbidden" not in buckets:
            c.create_bucket(Bucket="controller-lab-forbidden")
        break
    except Exception:
        if attempt==29: raise RuntimeError("MinIO readiness failed") from None
        time.sleep(2)
PY
rm /run/vzctl-lab-provider-stage/admin.json
echo 'Provider setup complete; administrator values were not printed.'
