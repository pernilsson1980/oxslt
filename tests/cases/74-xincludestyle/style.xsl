<!-- xincludestyle: XIncludes in the stylesheet are processed before it's
     compiled, but not in its imports, and not in documents document() loads. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xi="http://www.w3.org/2001/XInclude">
<xsl:import href="imp.xsl"/>
<xi:include href="part.xml"/>
<xsl:template match="/"><r><xsl:call-template name="part"/>|<xsl:call-template name="imp"/>|<xsl:value-of select="document('loaded.xml')"/></r></xsl:template>
</xsl:stylesheet>
