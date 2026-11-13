# The cloud engine's Cloud Run image, built from the `fanta-worker` crate; the
# binary and the image keep the name `fanta-engine`.
# .github/workflows/engine-image.yml builds the binary on ubuntu-24.04 and
# passes the folder holding it as the context, so the base must keep the same
# glibc.
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libfontconfig1 libfreetype6 \
    && rm -rf /var/lib/apt/lists/*

COPY fanta-engine /usr/local/bin/fanta-engine

USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/fanta-engine"]
CMD ["serve"]
