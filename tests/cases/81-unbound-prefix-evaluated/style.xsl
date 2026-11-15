<!-- Evaluated, an unbound function prefix stops the transform (exit 10). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><r><xsl:value-of select="stbl:convertLength(1)"/></r></xsl:template>
</xsl:stylesheet>
