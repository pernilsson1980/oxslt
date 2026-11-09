<?xml version="1.0"?>
<!-- EXSLT dates and times, as libexslt implements them. SOURCE_DATE_EPOCH
     (env) fixes the current date and time. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:date="http://exslt.org/dates-and-times" extension-element-prefixes="date">
  <xsl:output method="text"/>
  <xsl:template match="/">
    <xsl:text>now: </xsl:text><xsl:value-of select="date:date-time()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:date()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:time()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:year()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:month-name()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:day-name()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:week-in-year()"/>
    <xsl:text> | </xsl:text><xsl:value-of select="date:leap-year()"/>
    <xsl:text>&#10;available: </xsl:text>
    <xsl:value-of select="function-available('date:date-time')"/>
    <xsl:value-of select="function-available('date:day-name')"/>
    <xsl:for-each select="dates/d">
      <xsl:variable name="d" select="string(.)"/>
      <xsl:text>&#10;[</xsl:text><xsl:value-of select="$d"/><xsl:text>]</xsl:text>
      <xsl:text> date=</xsl:text><xsl:value-of select="date:date($d)"/>
      <xsl:text> time=</xsl:text><xsl:value-of select="date:time($d)"/>
      <xsl:text> year=</xsl:text><xsl:value-of select="date:year($d)"/>
      <xsl:text> leap=</xsl:text><xsl:value-of select="date:leap-year($d)"/>
      <xsl:text> mon=</xsl:text><xsl:value-of select="date:month-in-year($d)"/>
      <xsl:text> mname=</xsl:text><xsl:value-of select="date:month-name($d)"/>
      <xsl:text> mabbr=</xsl:text><xsl:value-of select="date:month-abbreviation($d)"/>
      <xsl:text> wiy=</xsl:text><xsl:value-of select="date:week-in-year($d)"/>
      <xsl:text> wim=</xsl:text><xsl:value-of select="date:week-in-month($d)"/>
      <xsl:text> diy=</xsl:text><xsl:value-of select="date:day-in-year($d)"/>
      <xsl:text> dim=</xsl:text><xsl:value-of select="date:day-in-month($d)"/>
      <xsl:text> dowim=</xsl:text><xsl:value-of select="date:day-of-week-in-month($d)"/>
      <xsl:text> diw=</xsl:text><xsl:value-of select="date:day-in-week($d)"/>
      <xsl:text> dname=</xsl:text><xsl:value-of select="date:day-name($d)"/>
      <xsl:text> dabbr=</xsl:text><xsl:value-of select="date:day-abbreviation($d)"/>
      <xsl:text> h=</xsl:text><xsl:value-of select="date:hour-in-day($d)"/>
      <xsl:text> m=</xsl:text><xsl:value-of select="date:minute-in-hour($d)"/>
      <xsl:text> s=</xsl:text><xsl:value-of select="date:second-in-minute($d)"/>
    </xsl:for-each>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
