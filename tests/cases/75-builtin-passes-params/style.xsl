<!-- libxslt's built-in template rule passes the caller's parameters on to
     the children's templates (XSLT 2.0's rule; XSLT 1.0's drops them), as
     fence-agents' fence2man.xsl relies on: no template matches "group". -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output method="text"/>
<xsl:template match="/"><xsl:apply-templates select="doc/group"><xsl:with-param name="show" select="'getopt'"/></xsl:apply-templates>|<xsl:apply-templates select="doc"><xsl:with-param name="show" select="'root'"/></xsl:apply-templates></xsl:template>
<xsl:template match="item"><xsl:param name="show" select="'none'"/>[<xsl:value-of select="concat(., ':', $show)"/>]</xsl:template>
<xsl:template match="text()"><xsl:param name="show" select="'none'"/>(<xsl:value-of select="$show"/>)</xsl:template>
</xsl:stylesheet>
