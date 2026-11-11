<?xml version="1.0"?>
<!-- xsl:number that formats nothing still inserts an empty text node (xsltCopyTextString), so elements stay open: an unknown function in value, a count that matches nothing, an unknown level. A failed value writes nothing and isn't an error. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><out><a><xsl:number value="nofunc()"/></a><b><xsl:number count="nomatch"/></b><c><xsl:number level="bogus"/></c><d>x<xsl:number value="nofunc()"/></d></out></xsl:template>
</xsl:stylesheet>
