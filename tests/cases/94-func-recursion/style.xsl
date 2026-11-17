<!-- Recursion without end stops at the recursion limit (exit 10). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:func="http://exslt.org/functions" xmlns:my="urn:my" extension-element-prefixes="func">
<func:function name="my:f"><xsl:param name="n"/><func:result select="my:f($n + 1)"/></func:function>
<xsl:template match="/"><r><xsl:value-of select="my:f(1)"/></r></xsl:template>
</xsl:stylesheet>
