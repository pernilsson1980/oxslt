<!-- A second func:result run in one call fails it. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:func="http://exslt.org/functions" xmlns:my="urn:my" extension-element-prefixes="func">
<func:function name="my:f"><xsl:for-each select="/|/doc"><func:result select="1"/></xsl:for-each></func:function>
<xsl:template match="/"><r><xsl:value-of select="my:f()"/></r></xsl:template>
</xsl:stylesheet>
