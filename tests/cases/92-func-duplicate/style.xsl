<!-- Two functions of one name in a stylesheet (and its includes) are a compile error (exit 5). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:func="http://exslt.org/functions" xmlns:my="urn:my" extension-element-prefixes="func">
<func:function name="my:f"><func:result select="1"/></func:function>
<func:function name="my:f"><func:result select="2"/></func:function>
<xsl:template match="/"><r><xsl:value-of select="my:f()"/></r></xsl:template>
</xsl:stylesheet>
