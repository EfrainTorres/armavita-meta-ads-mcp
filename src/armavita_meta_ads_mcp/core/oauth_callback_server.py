"""Local OAuth callback server used by Meta login flow."""


import json
import os
import socket
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Optional
from urllib.parse import parse_qs, urlparse

from .media_helpers import logger


CALLBACK_SERVER_TIMEOUT = 180

# Shared state updated by callback requests.
token_container: dict = {}



def reset_token_container() -> None:
    token_container.clear()
    token_container.update(
        {
            "token": None,
            "expires_in": None,
            "meta_user_id": None,
            "auth_code": None,
            "state": None,
            "redirect_uri": None,
            "status": "pending",
            "error": None,
            "timestamp": None,
        }
    )


reset_token_container()


_callback_lock = threading.Lock()
_callback_thread: Optional[threading.Thread] = None
_callback_httpd: Optional[HTTPServer] = None
_callback_port: Optional[int] = None
_shutdown_timer: Optional[threading.Timer] = None


class CallbackHandler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        parsed = urlparse(self.path)
        if parsed.path == "/callback":
            self._handle_callback(parsed)
            return
        if parsed.path == "/token":
            self._json_response({"status": "success", "data": token_container})
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, fmt, *args):  # noqa: D401, ANN001
        _ = (fmt, args)
        return

    def _json_response(self, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _html_response(self, html: str) -> None:
        body = html.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _handle_callback(self, parsed_url) -> None:
        params = parse_qs(parsed_url.query)
        auth_code = params.get("code", [None])[0]
        state = params.get("state", [None])[0]
        auth_error = params.get("error", [None])[0]
        auth_error_description = params.get("error_description", [None])[0]

        if auth_error:
            primary_text = auth_error_description or auth_error
            token_container.update(
                {
                    "status": "error",
                    "error": primary_text,
                    "state": state,
                    "timestamp": time.time(),
                }
            )
            self._html_response(
                "<html><body><h1>Authorization failed</h1><p>"
                + primary_text
                + "</p></body></html>"
            )
            _shutdown_async()
            return

        if not auth_code:
            token_container.update(
                {
                    "status": "error",
                    "error": "missing_code_or_error",
                    "timestamp": time.time(),
                }
            )
            self._html_response(
                "<html><body><h1>Authorization failed</h1><p>No auth code provided.</p></body></html>"
            )
            _shutdown_async()
            return

        token_container.update(
            {
                "auth_code": auth_code,
                "state": state,
                "status": "exchanging",
                "error": None,
                "timestamp": time.time(),
            }
        )

        try:
            from .auth_state import complete_oauth_from_auth_code

            result = complete_oauth_from_auth_code(
                auth_code,
                redirect_uri=token_container.get("redirect_uri"),
                persist_token=True,
            )
            if result.get("success"):
                token_info = result.get("token_info")
                token_container.update(
                    {
                        "token": getattr(token_info, "meta_access_token", None),
                        "expires_in": getattr(token_info, "expires_in", None),
                        "meta_user_id": getattr(token_info, "meta_user_id", None),
                        "status": "success",
                        "error": None,
                        "timestamp": time.time(),
                    }
                )
                self._html_response(
                    "<html><body><h1>Authorization complete</h1><p>You can close this window.</p>"
                    "<script>setTimeout(()=>window.close(),1200);</script></body></html>"
                )
            else:
                error_code = result.get("error", "oauth_completion_failed")
                token_container.update(
                    {
                        "status": "error",
                        "error": error_code,
                        "timestamp": time.time(),
                    }
                )
                self._html_response(
                    "<html><body><h1>Authorization failed</h1><p>"
                    + str(error_code)
                    + "</p></body></html>"
                )
        except Exception as exc:  # noqa: BLE001
            logger.exception("OAuth completion callback failed: %s", exc)
            token_container.update(
                {
                    "status": "error",
                    "error": str(exc),
                    "timestamp": time.time(),
                }
            )
            self._html_response(
                "<html><body><h1>Authorization failed</h1><p>Internal error while completing login.</p></body></html>"
            )

        _shutdown_async()



def _choose_port(start_port: int = 8080, attempts: int = 10) -> int:
    port = start_port
    for _ in range(attempts):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            try:
                probe.bind(("localhost", port))
            except OSError:
                port += 1
                continue
            return port
    raise RuntimeError(f"Unable to allocate callback port after {attempts} attempts")



def _server_loop(port: int) -> None:
    global _callback_httpd
    try:
        httpd = HTTPServer(("localhost", port), CallbackHandler)
        _callback_httpd = httpd
        logger.info("Callback server listening on http://localhost:%s", port)
        httpd.serve_forever()
    except Exception as exc:  # noqa: BLE001
        logger.exception("Callback server loop crashed: %s", exc)



def _shutdown_async() -> None:
    threading.Thread(target=shutdown_callback_server, daemon=True).start()



def shutdown_callback_server() -> None:
    global _callback_thread, _callback_httpd, _callback_port, _shutdown_timer

    with _callback_lock:
        if _shutdown_timer is not None:
            _shutdown_timer.cancel()
            _shutdown_timer = None

        if _callback_httpd is not None:
            try:
                _callback_httpd.shutdown()
                _callback_httpd.server_close()
            except Exception as exc:  # noqa: BLE001
                logger.warning("Error while closing callback server: %s", exc)

        if _callback_thread is not None and _callback_thread.is_alive():
            _callback_thread.join(timeout=3)

        _callback_thread = None
        _callback_httpd = None
        _callback_port = None



def start_callback_server() -> int:
    global _callback_thread, _callback_port, _shutdown_timer

    if os.environ.get("META_ADS_DISABLE_CALLBACK_SERVER"):
        raise RuntimeError("Callback server disabled via META_ADS_DISABLE_CALLBACK_SERVER")

    with _callback_lock:
        reset_token_container()

        if _callback_port is not None:
            token_container["redirect_uri"] = f"http://localhost:{_callback_port}/callback"
            return _callback_port

        port = _choose_port()
        _callback_port = port
        token_container["redirect_uri"] = f"http://localhost:{port}/callback"

        _callback_thread = threading.Thread(target=_server_loop, args=(port,), daemon=True)
        _callback_thread.start()

        # short delay to let socket bind and thread initialize
        time.sleep(0.3)

        def timed_shutdown() -> None:
            logger.info("Callback server timed out after %ss", CALLBACK_SERVER_TIMEOUT)
            shutdown_callback_server()

        _shutdown_timer = threading.Timer(CALLBACK_SERVER_TIMEOUT, timed_shutdown)
        _shutdown_timer.daemon = True
        _shutdown_timer.start()

        return port
