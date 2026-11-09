# Reference environment: Debian's xsltproc (libxslt 1.1.35) plus offline
# DocBook DTDs and stylesheets. Used to record expected output for
# third-party stylesheets, which never run directly on a developer machine.
#
#   docker build --platform linux/amd64 -t oxslt-reference tests/reference
FROM --platform=linux/amd64 debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends xsltproc docbook-xml docbook-xsl \
 && rm -rf /var/lib/apt/lists/*

ENV XML_CATALOG_FILES=/etc/xml/catalog
