# check=skip=InvalidDefaultArgInFrom
# Sally's tools on top of the repository's image, passed by vz as BASE; no
# default, so this file does not build alone.
ARG BASE
FROM ${BASE}
RUN mkdir -p /etc/sally && touch /etc/sally/marker
