<?xml version="1.0"?>
<!-- xsl:for-each over a fragment isn't a node-set: reported, the transform goes on, exit 9. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:exsl="http://exslt.org/common" xmlns:set="http://exslt.org/sets" exclude-result-prefixes="exsl set"><xsl:variable name="r"><a>1</a></xsl:variable><xsl:template match="/"><out><xsl:for-each select="$r">x</xsl:for-each></out></xsl:template>
</xsl:stylesheet>
