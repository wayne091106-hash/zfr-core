"""ZFR Relay — serverless egress with a fresh IP on every cold boot.

The relay is a tiny authenticated HTTP forwarder that runs as a Modal web
endpoint. ZFR Core wraps each upstream request into a JSON envelope
(`build_relay_envelope` in src/router.rs) and POSTs it here; the relay performs
the request from the container's own egress IP and streams the upstream status,
a small allow-list of headers and the body straight back — so 429 / Retry-After
semantics survive the hop and keep driving the normal cooldown pool.

Deploy (Modal Starter plan is free, includes $30/month credits):

    pip install modal fastapi httpx   # fastapi/httpx are also needed locally to deploy
    modal setup
    modal secret create zfr-relay RELAY_KEY=<random-secret>
    modal deploy modal/relay.py

Then add the printed endpoint to the ZFR Core egress pool:

    zfr serve --egress auto,relay://https://<endpoint>.modal.run#key=<RELAY_KEY>

Lifecycle: the container idles out after `scaledown_window` seconds (nothing is
billed while it is down) and the next request cold-boots a fresh container,
practically always with a fresh egress IP from Modal's cloud pool.
"""

from typing import AsyncIterator

import fastapi
import httpx
import modal
from fastapi.responses import JSONResponse, StreamingResponse

app = modal.App("zfr-relay")

image = modal.Image.debian_slim(python_version="3.12").pip_install(
    "httpx==0.28.*",
    "fastapi==0.115.*",
)

# Seconds the container stays alive after its last request. Keep this BELOW
# ZFR Core's RELAY_QUOTA_COOLDOWN_SECS (90s, src/egress.rs) so that a quota-429
# cooldown expires only after the container (and its burned IP) is gone.
SCALEDOWN_WINDOW_SECS = 60


@app.function(
    image=image,
    secrets=[modal.Secret.from_name("zfr-relay")],
    scaledown_window=SCALEDOWN_WINDOW_SECS,
    max_containers=1,
    timeout=900,
)
@modal.fastapi_endpoint(method="POST")
async def relay(request: fastapi.Request) -> fastapi.Response:
    import os

    expected = os.environ.get("RELAY_KEY", "")
    if expected and request.headers.get("x-zfr-relay-key") != expected:
        return JSONResponse({"error": "invalid relay key"}, status_code=401)

    try:
        spec = await request.json()
        method = str(spec["method"]).upper()
        url = str(spec["url"])
        headers = {str(k): str(v) for k, v in (spec.get("headers") or {}).items()}
        body = spec.get("body")
    except Exception:
        return JSONResponse({"error": "malformed request envelope"}, status_code=400)

    if not url.startswith(("http://", "https://")):
        return JSONResponse({"error": "only http(s) urls are relayed"}, status_code=400)

    client = httpx.AsyncClient(
        timeout=httpx.Timeout(connect=30.0, read=840.0, write=30.0, pool=30.0),
        follow_redirects=True,
    )
    try:
        upstream = await client.send(
            client.build_request(method, url, headers=headers, content=body),
            stream=True,
        )
    except httpx.HTTPError as exc:
        await client.aclose()
        return JSONResponse(
            {"error": f"relay transport error: {exc.__class__.__name__}: {exc}"},
            status_code=502,
        )

    # Pass through exactly what ZFR Core's cooldown logic consumes upstream:
    # the status code, Retry-After and rate-limit headers. content-type is
    # re-applied via `media_type` so it is never duplicated.
    allow = {"retry-after", "date"}
    passthrough = {
        k: v
        for k, v in upstream.headers.items()
        if k.lower() in allow
        or k.lower().startswith(("x-ratelimit", "ratelimit", "x-request-id"))
    }
    return StreamingResponse(
        _stream_and_cleanup(upstream, client),
        status_code=upstream.status_code,
        headers=passthrough,
        media_type=upstream.headers.get("content-type"),
    )


async def _stream_and_cleanup(
    upstream: httpx.Response, client: httpx.AsyncClient
) -> AsyncIterator[bytes]:
    try:
        async for chunk in upstream.aiter_raw():
            yield chunk
    finally:
        await upstream.aclose()
        await client.aclose()
