# Predefined execution requirement dictionary fragments for Bazel workers.
# Users can merge these into their action execution_requirements.
# See https://bazel.build/remote/creating

SUPPORTS_WORKERS = {"supports-workers": "1"}
SUPPORTS_MULTIPLEX_WORKERS = {"supports-multiplex-workers": "1"}
REQUIRES_JSON = {"requires-worker-protocol": "json"}
REQUIRES_PROTO = {"requires-worker-protocol": "proto"}
