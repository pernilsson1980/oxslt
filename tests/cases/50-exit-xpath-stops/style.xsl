<?xml version="1.0"?>
<!-- A failed XPath evaluation in an instruction stops the transform: libxslt's XSLT_STATE_STOPPED, exit 10 (an undefined variable reports an error first, but stopping decides). -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><xsl:value-of select="$nope"/></out></xsl:template>
</xsl:stylesheet>
