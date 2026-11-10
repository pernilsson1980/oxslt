<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:include href="sub.xsl"/>
  <xsl:variable name="data" select="string(document('data.xml')/d)"/>
</xsl:stylesheet>
