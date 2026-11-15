<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output method="text"/>
<xsl:template match="/"><xsl:for-each select="doc/item"><xsl:value-of select="concat(position(), '=', ., '&#10;')"/></xsl:for-each></xsl:template>
</xsl:stylesheet>
