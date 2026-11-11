<?xml version="1.0"?>
<!-- A path step on a result tree fragment is a type error in XSLT 1.0, and in libxml2: the evaluation fails and the transform stops (exit 10). exsl:node-set() would have been needed. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:exsl="http://exslt.org/common" xmlns:set="http://exslt.org/sets" exclude-result-prefixes="exsl set"><xsl:variable name="r"><a>1</a></xsl:variable><xsl:template match="/"><out><xsl:value-of select="$r/a"/></out></xsl:template>
</xsl:stylesheet>
