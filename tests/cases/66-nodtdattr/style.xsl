<!-- nodtdattr: entities are still substituted, default attributes aren't added. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><r a="{doc/@a}"><xsl:value-of select="doc"/></r></xsl:template>
</xsl:stylesheet>
