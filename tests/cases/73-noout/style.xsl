<!-- noout: the transform runs (its messages print) but writes no result. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
<xsl:template match="/"><xsl:message>ran</xsl:message><r/></xsl:template>
</xsl:stylesheet>
