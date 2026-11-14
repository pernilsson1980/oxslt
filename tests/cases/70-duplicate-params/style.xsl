<!-- A second command-line parameter of a name is an error (exit 9), for a
     declared parameter (p) and an undeclared one (q) alike. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:param name="p"/>
<xsl:template match="/"><r><xsl:value-of select="$p"/></r></xsl:template>
</xsl:stylesheet>
