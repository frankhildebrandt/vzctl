#!/usr/bin/env python3
"""Real DB/S3/OIDC consumer; output contains booleans/error classes, no credentials."""
import hashlib
import json
import os
import secrets
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path


def binding(name):
    root = Path(os.environ.get("BINDING_ROOT", "/var/lib/vzctl/bindings")) / name
    if os.environ.get("BINDING_MODE") == "kubernetes":
        return {p.name: p.read_text() for p in root.iterdir() if not p.name.startswith(".")}
    value = json.loads((root / "binding.json").read_text())
    return dict(value["public"], **value["secrets"])


def postgres():
    import pg8000.dbapi
    b = binding("db")
    c = pg8000.dbapi.connect(host=b["host"], port=int(b["port"]), database=b["database"],
                            user=b["user"], password=b["password"], timeout=10)
    try:
        cursor = c.cursor()
        cursor.execute("CREATE TABLE IF NOT EXISTS lab_proof (id text PRIMARY KEY, value text NOT NULL)")
        cursor.execute("INSERT INTO lab_proof VALUES ('proof','controller-lab') "
                       "ON CONFLICT (id) DO UPDATE SET value=excluded.value")
        c.commit()
        cursor.execute("SELECT value FROM lab_proof WHERE id='proof'")
        assert cursor.fetchone()[0] == "controller-lab"
        cursor.execute("SELECT rolsuper,rolcreatedb,rolcreaterole FROM pg_roles WHERE rolname=current_user")
        assert tuple(cursor.fetchone()) == (False, False, False)
    finally:
        c.close()


def s3():
    import boto3
    from botocore.config import Config
    b = binding("files")
    c = boto3.client("s3", endpoint_url=b["endpoint"], region_name=b["region"],
                     aws_access_key_id=b["accessKey"], aws_secret_access_key=b["secretKey"],
                     config=Config(connect_timeout=5, read_timeout=10,
                                   retries={"max_attempts": 1}, s3={"addressing_style": "path"}))
    key = "lab-proof/" + os.environ.get("PROBE_NAME", "vm")
    c.put_object(Bucket=b["bucket"], Key=key, Body=b"controller-lab")
    obj = c.get_object(Bucket=b["bucket"], Key=key)
    assert obj["Body"].read() == b"controller-lab"
    obj["Body"].close()
    # A different bucket must not become accessible through this binding.
    try:
        c.list_objects_v2(Bucket="controller-lab-forbidden", MaxKeys=1)
    except c.exceptions.ClientError as error:
        assert error.response["Error"]["Code"] in ("AccessDenied", "AllAccessDisabled")
    else:
        raise AssertionError("cross-bucket access unexpectedly allowed")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def oidc():
    import base64
    import jwt
    b = binding("login")
    context = ssl.create_default_context(cafile=os.environ.get("LAB_CA", "/opt/vzctl-lab/ca.crt"))
    opener = urllib.request.build_opener(urllib.request.HTTPSHandler(context=context), NoRedirect())
    issuer = b["issuer"]
    with opener.open(issuer + "/.well-known/openid-configuration", timeout=10) as response:
        discovery = json.load(response)
    assert discovery["issuer"] == issuer
    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    state, nonce = secrets.token_hex(16), secrets.token_hex(16)
    redirect = "https://docker.svc.controller-lab.vz.test/callback"
    params = dict(response_type="code", client_id=b["clientID"], redirect_uri=redirect,
                  scope="openid profile email", state=state, nonce=nonce,
                  code_challenge=challenge, code_challenge_method="S256")
    with opener.open(discovery["authorization_endpoint"] + "?" + urllib.parse.urlencode(params),
                     timeout=10) as response:
        assert response.status == 200
    params.pop("response_type")
    params["username"] = "lab"
    try:
        opener.open(issuer + "/login", urllib.parse.urlencode(params).encode(), timeout=10)
    except urllib.error.HTTPError as response:
        assert response.code in (302, 303, 307)
        location = urllib.parse.urlparse(response.headers["Location"])
        query = urllib.parse.parse_qs(location.query)
        assert query["state"] == [state]
        code = query["code"][0]
    else:
        raise AssertionError("login did not redirect")
    token_request = dict(grant_type="authorization_code", code=code, redirect_uri=redirect,
                         client_id=b["clientID"], client_secret=b["clientSecret"], code_verifier=verifier)
    with opener.open(discovery["token_endpoint"], urllib.parse.urlencode(token_request).encode(),
                     timeout=10) as response:
        tokens = json.load(response)
    key = jwt.PyJWKClient(discovery["jwks_uri"], ssl_context=context, timeout=10).get_signing_key_from_jwt(tokens["id_token"])
    claims = jwt.decode(tokens["id_token"], key.key, algorithms=["RS256"],
                        audience=b["clientID"], issuer=issuer)
    assert claims["nonce"] == nonce and claims["preferred_username"] == "lab"
    req = urllib.request.Request(discovery["userinfo_endpoint"],
                                 headers={"Authorization": "Bearer " + tokens["access_token"]})
    with opener.open(req, timeout=10) as response:
        assert json.load(response)["sub"] == claims["sub"]


def run():
    results = {}
    for name, check in [("postgres", postgres), ("s3", s3), ("oidc", oidc)]:
        try:
            check()
            results[name] = True
        except Exception as error:
            results[name] = type(error).__name__
    return results


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        result = run()
        body = json.dumps(result).encode()
        self.send_response(200 if all(v is True for v in result.values()) else 503)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    if sys.argv[1:] == ["serve"]:
        HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
    else:
        result = run()
        print(json.dumps(result))
        sys.exit(0 if all(v is True for v in result.values()) else 1)
