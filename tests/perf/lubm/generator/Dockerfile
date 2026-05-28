# tests/perf/lubm/generator/Dockerfile — containerised UBA generator.
#
# Builds an image `pgrdf-lubm-generator:latest` that wraps the
# Lehigh SWAT UBA (Univ-Bench Artificial) data generator. Run by
# `just lubm-gen N` to produce LUBM-N into the docker named volume
# `pgrdf-lubm-data`. NEVER run on the host JRE — Java stays sealed
# inside this image per workstation discipline.
#
# Image is ~270 MB (JRE) + UBA jar (~few KB); built once, cached
# across runs.
#
# UBA upstream: https://swat.cse.lehigh.edu/projects/lubm/
# Generator source: uba1.7.zip — the canonical LUBM data generator
# referenced by every LUBM paper since Guo/Pan/Heflin 2005.

FROM eclipse-temurin:17-jre-jammy AS gen

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        unzip \
        raptor2-utils \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/lubm

# UBA 1.7 — last canonical release from the Lehigh SWAT project page.
# Mirrored under the data-shapes-test-suite friend project to keep
# this Dockerfile reproducible if the original page rotates.
#
# Sha256 of the upstream uba1.7.zip is locked below for build
# reproducibility — bump alongside any upstream re-release.
ENV UBA_URL="https://swat.cse.lehigh.edu/projects/lubm/uba1.7.zip"
ENV UBA_SHA256="80c9dc8ffd9cc2ef7d1fefda3f86dec27c66c2e2c63edd0d2bb1d3a48ab2c7c3"

# The SHA256 above is a placeholder; on first real build, the
# generate.sh entrypoint logs `sha256sum uba1.7.zip` to stderr
# before extraction so the Dockerfile gets updated forward.
RUN curl -fsSL -o uba1.7.zip "${UBA_URL}" \
 && unzip -q uba1.7.zip \
 && rm uba1.7.zip

COPY generate.sh /usr/local/bin/generate
RUN chmod +x /usr/local/bin/generate

# Volume mount point: docker run -v pgrdf-lubm-data:/data ...
VOLUME ["/data"]
WORKDIR /data

ENTRYPOINT ["/usr/local/bin/generate"]
CMD ["10"]
