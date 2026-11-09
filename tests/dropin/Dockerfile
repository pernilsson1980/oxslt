# The drop-in test image: Debian bookworm with the build dependencies and
# sources of the packages in packages.txt, and the comparing wrapper installed
# as /usr/bin/xsltproc. Built with network access (scripts/dropin.sh); the
# builds themselves run offline with a read-only root.
FROM debian:bookworm

RUN sed -i 's/^Types: deb$/Types: deb deb-src/' /etc/apt/sources.list.d/debian.sources \
 && apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential devscripts fakeroot dpkg-dev xsltproc docbook-xsl docbook-xml \
 && rm -rf /var/lib/apt/lists/*

COPY packages.txt /dropin/packages.txt

# Build dependencies; a package whose dependencies can't be installed is
# recorded and skipped.
RUN apt-get update \
 && for p in $(grep -v '^#' /dropin/packages.txt); do \
      DEBIAN_FRONTEND=noninteractive apt-get build-dep -y --no-install-recommends "$p" \
        || echo "$p" >> /dropin/build-dep-failed; \
    done \
 && rm -rf /var/lib/apt/lists/* \
 && touch /dropin/build-dep-failed

# Sources, unpacked fresh for each run.
RUN apt-get update && mkdir -p /dropin/src && cd /dropin/src \
 && for p in $(grep -v '^#' /dropin/packages.txt); do \
      apt-get source --download-only "$p" || echo "$p" >> /dropin/source-failed; \
    done \
 && rm -rf /var/lib/apt/lists/* \
 && touch /dropin/source-failed

# The wrapper takes xsltproc's place; oxslt is mounted at /opt/oxslt.
RUN dpkg-divert --add --rename --divert /usr/bin/xsltproc.real /usr/bin/xsltproc \
 && mkdir -p /opt/oxslt
COPY xsltproc-wrapper.sh /usr/bin/xsltproc
COPY run.sh /dropin/run.sh
RUN chmod 755 /usr/bin/xsltproc /dropin/run.sh
