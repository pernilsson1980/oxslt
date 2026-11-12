<?xml version="1.0"?>
<!-- EXSLT date:add, date:add-duration, date:difference, date:seconds, date:duration and date:sum, ported from libexslt: every date (of each type, with edge cases and invalid ones) against every duration and date. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:date="http://exslt.org/dates-and-times" extension-element-prefixes="date">
<xsl:output method="text"/>
<xsl:template match="/t">
<xsl:for-each select="d"><xsl:variable name="d" select="."/>
  <xsl:value-of select="concat('S ', $d, ' = ', date:seconds($d), '&#10;')"/>
  <xsl:for-each select="../u"><xsl:value-of select="concat('A ', $d, ' + ', ., ' = ', date:add($d, .), '&#10;')"/></xsl:for-each>
  <xsl:for-each select="../d"><xsl:value-of select="concat('D ', $d, ' .. ', ., ' = ', date:difference($d, .), '&#10;')"/></xsl:for-each>
</xsl:for-each>
<xsl:for-each select="u"><xsl:variable name="u" select="."/>
  <xsl:value-of select="concat('S ', $u, ' = ', date:seconds($u), '&#10;')"/>
  <xsl:for-each select="../u"><xsl:value-of select="concat('U ', $u, ' + ', ., ' = ', date:add-duration($u, .), '&#10;')"/></xsl:for-each>
</xsl:for-each>
<xsl:for-each select="n"><xsl:value-of select="concat('N ', ., ' = ', date:duration(.), '&#10;')"/></xsl:for-each>
<xsl:value-of select="concat('SUM ', date:sum(u[position() &lt; 4]), ' | ', date:sum(u[1] | u[5] | u[7]), ' | ', date:sum(u[2]|u[4]), ' | ', date:sum(u[100]), ' | ', date:sum(u[32]), '&#10;')"/>
</xsl:template>
</xsl:stylesheet>
