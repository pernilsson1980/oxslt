<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output encoding="UTF-8"/>
<xsl:template match="/"><r><xsl:value-of select="doc"/>|<xsl:value-of select="string-length(doc)"/></r></xsl:template>
</xsl:stylesheet>
