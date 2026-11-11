<?xml version="1.0"?>
<!-- Circular globals aren't a compile error in libxslt; computing them stops the transform: exit 10. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:variable name="a" select="$b"/><xsl:variable name="b" select="$a"/><xsl:template match="/"><out/></xsl:template>
</xsl:stylesheet>
