<!-- A command-line parameter is evaluated when the transform starts, even
     for a declared parameter that's never used; a failure stops it (exit 10). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:param name="p"/>
<xsl:template match="/"><r/></xsl:template>
</xsl:stylesheet>
