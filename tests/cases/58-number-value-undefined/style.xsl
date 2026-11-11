<?xml version="1.0"?>
<!-- An undefined variable in xsl:number value is reported (exit 9) but stops nothing. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><xsl:number value="$nope"/></out></xsl:template>
</xsl:stylesheet>
