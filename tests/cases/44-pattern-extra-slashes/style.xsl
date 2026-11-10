<?xml version="1.0"?>
<!-- libxslt's pattern compiler doesn't skip extra slashes as libxml2's
     XPath compiler does: match="///b" is a compile error. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="///b">B</xsl:template>
</xsl:stylesheet>
