<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/>
  <xsl:template match="/d">
    <xsl:value-of select="sum(n[number(.)=number(.)])"/>|<xsl:value-of select="count(n)"/>|<xsl:value-of select="round(2.5)"/>,<xsl:value-of select="round(-2.5)"/>,<xsl:value-of select="floor(-1.5)"/>,<xsl:value-of select="ceiling(1.2)"/>
|<xsl:value-of select="1 div 0"/>,<xsl:value-of select="-1 div 0"/>,<xsl:value-of select="0 div 0"/>,<xsl:value-of select="number('x')"/>,<xsl:value-of select="10 mod 3"/>,<xsl:value-of select="-10 mod 3"/>
|<xsl:value-of select="normalize-space(s)"/>|<xsl:value-of select="translate('abcabc','ab','X')"/>|<xsl:value-of select="substring('12345', 1.5, 2.6)"/>|<xsl:value-of select="substring-before('a/b/c','/')"/>|<xsl:value-of select="substring-after('a/b/c','/')"/>
|<xsl:value-of select="string-length(normalize-space(s))"/>|<xsl:value-of select="contains(s,'World')"/>|<xsl:value-of select="starts-with('abc','ab')"/>|<xsl:value-of select="boolean(e)"/>|<xsl:value-of select="boolean(string(e))"/>|<xsl:value-of select="not(n = 'abc')"/>
|<xsl:value-of select="n = -2"/>|<xsl:value-of select="n != -2"/>|<xsl:value-of select="n &gt; 3"/>|<xsl:value-of select="'2' = 2.0"/>|<xsl:value-of select="0.1 + 0.2"/>|<xsl:value-of select="1e0 * 1000000000000000000000"/>|<xsl:value-of select="-0"/>
|<xsl:value-of select="format-number(1234567.891,'#,##0.00')"/>|<xsl:value-of select="format-number(0.5,'0%')"/>|<xsl:value-of select="format-number(-3,'000;(000)')"/>|<xsl:value-of select="name(*[last()])"/>|<xsl:value-of select="local-name(/*)"/>
<xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
