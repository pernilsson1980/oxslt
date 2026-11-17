<!-- More arguments than the function has parameters fail the call: the transform stops (exit 10). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:func="http://exslt.org/functions" xmlns:my="urn:my" extension-element-prefixes="func">
<func:function name="my:f"><xsl:param name="a"/><func:result select="$a"/></func:function>
<xsl:template match="/"><r><xsl:value-of select="my:f(1, 2)"/></r></xsl:template>
</xsl:stylesheet>
