# milkfloat's sandboxes: fctl's image, with the solvers the benchmarks
# compare against and maturin for the Python bindings.
FROM ghcr.io/hyperstructure/sandbox:0.23

USER root
RUN apt-get update \
  && apt-get install -y --no-install-recommends coinor-cbc glpk-utils \
  && rm -rf /var/lib/apt/lists/*
USER agent

RUN uv tool install maturin==1.9.6
