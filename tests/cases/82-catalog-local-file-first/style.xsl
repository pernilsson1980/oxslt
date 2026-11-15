<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><r from="{doc/@from}"><xsl:value-of select="document('other.xml')/doc/@from"/></r></xsl:template>
</xsl:stylesheet>
