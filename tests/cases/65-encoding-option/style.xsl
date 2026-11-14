<!-- xsltproc's encoding option: the input is read as ISO-8859-1, its
     declaration ignored, so the UTF-8 bytes become two characters each. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:output encoding="UTF-8"/>
<xsl:template match="/"><r><xsl:value-of select="."/>|<xsl:value-of select="string-length(.)"/></r></xsl:template>
</xsl:stylesheet>
